//! A small, bounded journal of recent errors (panics, `log::error!`, explicit reports) kept in
//! `errors.json` for the "!" badge on the settings gear. Not a general log. No UI dependencies:
//! the app subscribes to `Event`s and reads `snapshot()`.
//!
//! TODO: native crashes (SIGSEGV, SIGBUS, aborts in foreign code) are not captured; they would
//! need a signal handler that writes without allocating.

mod journal;
mod record;
mod retention;
mod storage;
mod text;

use std::backtrace::{Backtrace, BacktraceStatus};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, TryLockError};
use std::time::{Duration, Instant};

pub use journal::{Journal, Limits, Loaded, MAX_BYTES, MAX_RECORDS, Prune, StartReport};
pub use record::{Capture, ErrorRecord, Identity, JournalFile, Kind, records_to_markdown};
pub use retention::{Clock, Retention, SystemClock};
pub use storage::FILE_NAME;
pub use text::{format_utc, parse_location, relative_time};

/// Writes are coalesced over this window.
const DEBOUNCE: Duration = Duration::from_millis(500);
const PRUNE_EVERY: Duration = Duration::from_secs(3_600);

#[derive(Debug, Clone)]
pub struct Options {
    /// The IDE data dir; `errors.json` lives here.
    pub dir: PathBuf,
    pub app_version: String,
    pub build: String,
    pub retention: Retention,
    pub clock: Arc<dyn Clock>,
    pub limits: Limits,
}

/// Sent to subscribers from the journal's worker thread, never from the panic hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// Records or their seen state changed.
    Changed,
    /// Retention just removed this many records.
    Pruned(usize),
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    /// Newest first.
    pub records: Vec<ErrorRecord>,
    pub unseen: usize,
    pub retention: Retention,
    pub now: u64,
}

type Listener = Box<dyn Fn(Event) + Send + Sync>;

enum Message {
    Changed,
    /// Built on the worker: symbolizing a backtrace is slow.
    Capture(Capture, Option<Backtrace>),
    /// A panic record the hook could not add because the journal was busy.
    Add(Box<ErrorRecord>),
    Retention(Retention),
}

struct Shared {
    journal: Mutex<Journal>,
    ident: Identity,
    dir: PathBuf,
    clock: Arc<dyn Clock>,
    /// Bumped on every change; writes skip states older than the last one written.
    generation: AtomicU64,
    written: Mutex<u64>,
    seq: AtomicU64,
    tx: Sender<Message>,
    listeners: Mutex<Vec<Listener>>,
}

static SHARED: OnceLock<Shared> = OnceLock::new();

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn try_lock<T>(mutex: &Mutex<T>) -> Option<MutexGuard<'_, T>> {
    match mutex.try_lock() {
        Ok(guard) => Some(guard),
        Err(TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner()),
        Err(TryLockError::WouldBlock) => None,
    }
}

impl Shared {
    fn path(&self) -> PathBuf {
        storage::journal_path(&self.dir)
    }

    fn next_seq(&self) -> u64 {
        self.seq.fetch_add(1, Ordering::Relaxed)
    }

    /// Changes the journal and marks the new state for writing.
    fn mutate<R>(&self, change: impl FnOnce(&mut Journal) -> R) -> R {
        let mut journal = lock(&self.journal);
        let result = change(&mut journal);
        self.generation.fetch_add(1, Ordering::SeqCst);
        result
    }

    fn persist(&self) {
        let (generation, bytes) = {
            let journal = lock(&self.journal);
            (self.generation.load(Ordering::SeqCst), journal.to_bytes())
        };
        let mut written = lock(&self.written);
        if generation <= *written {
            return;
        }
        // Nowhere better to report a failed write of the error journal itself.
        if storage::write_atomic(&self.path(), &bytes).is_ok() {
            *written = generation;
        }
    }

    fn notify(&self, event: Event) {
        for listener in lock(&self.listeners).iter() {
            listener(event);
        }
    }

    fn send(&self, message: Message) {
        let _ = self.tx.send(message);
    }
}

/// Loads `errors.json` (resetting it on a new version or corruption), merges panic files left
/// by a crash, applies retention and marks the session as running. Call once, early in `main`.
pub fn init(options: Options) -> anyhow::Result<StartReport> {
    anyhow::ensure!(SHARED.get().is_none(), "the error journal is already initialized");
    text::warm_up();
    let now = options.clock.now();
    let ident = Identity {
        app_version: options.app_version,
        build: options.build,
        os: detect_os(),
        session_id: format!("{:x}{:04x}", now, std::process::id() & 0xffff),
        home: std::env::var("HOME").ok().filter(|h| h.len() > 1),
    };
    let loaded = storage::load(&options.dir);
    let (orphans, orphan_paths) = storage::panic_files(&options.dir);
    let (journal, report) = Journal::start(
        loaded,
        orphans,
        ident.clone(),
        options.retention,
        options.limits,
        now,
    );
    // Before anything else runs: `clean_shutdown = false` must be on disk to detect a crash.
    if storage::write_atomic(&storage::journal_path(&options.dir), &journal.to_bytes()).is_ok() {
        storage::remove_all(&orphan_paths);
    }
    let (tx, rx) = mpsc::channel();
    let shared = Shared {
        journal: Mutex::new(journal),
        ident,
        dir: options.dir,
        clock: options.clock,
        generation: AtomicU64::new(1),
        written: Mutex::new(1),
        seq: AtomicU64::new(1),
        tx,
        listeners: Mutex::new(Vec::new()),
    };
    if SHARED.set(shared).is_err() {
        anyhow::bail!("the error journal is already initialized");
    }
    std::thread::Builder::new()
        .name("error-journal".into())
        .spawn(move || {
            if let Some(shared) = SHARED.get() {
                worker(shared, &rx);
            }
        })?;
    Ok(report)
}

fn worker(shared: &Shared, rx: &Receiver<Message>) {
    let mut flush_at: Option<Instant> = None;
    let mut next_prune = Instant::now() + PRUNE_EVERY;
    loop {
        let deadline = flush_at.map_or(next_prune, |at| at.min(next_prune));
        let message = rx.recv_timeout(deadline.saturating_duration_since(Instant::now()));
        let mut changed = false;
        match message {
            Ok(Message::Changed) => changed = true,
            Ok(Message::Capture(mut capture, backtrace)) => {
                if let Some(backtrace) = backtrace {
                    capture.backtrace = backtrace.to_string();
                }
                let record =
                    ErrorRecord::build(capture, &shared.ident, shared.clock.now(), shared.next_seq());
                shared.mutate(|journal| journal.add(record));
                changed = true;
            }
            Ok(Message::Add(record)) => {
                shared.mutate(|journal| journal.add(*record));
                changed = true;
            }
            Ok(Message::Retention(retention)) => {
                let now = shared.clock.now();
                let removed = shared.mutate(|journal| journal.set_retention(retention, now));
                shared.notify(Event::Pruned(removed));
                changed = true;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        let now = Instant::now();
        if changed {
            flush_at.get_or_insert(now + DEBOUNCE);
            shared.notify(Event::Changed);
        }
        if now >= next_prune {
            next_prune = now + PRUNE_EVERY;
            flush_at.get_or_insert(now);
        }
        if flush_at.is_some_and(|at| now >= at) {
            flush_at = None;
            let clock_now = shared.clock.now();
            let removed = shared.mutate(|journal| journal.prune(clock_now, Prune::Periodic));
            shared.persist();
            if removed > 0 {
                shared.notify(Event::Changed);
            }
        }
    }
}

/// Chains a hook that records every panic (any thread) before the previous hook runs. The
/// record is written synchronously because the process may abort right after.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        record_panic(info);
        previous(info);
    }));
}

fn record_panic(info: &std::panic::PanicHookInfo<'_>) {
    let Some(shared) = SHARED.get() else { return };
    let payload = info.payload();
    let message = payload
        .downcast_ref::<&str>()
        .map(ToString::to_string)
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "Box<dyn Any>".into());
    let capture = Capture {
        kind: Some(Kind::Panic),
        message,
        location: info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column())),
        thread: std::thread::current().name().map(str::to_string),
        context: None,
        backtrace: Backtrace::force_capture().to_string(),
    };
    let now = shared.clock.now();
    let record = ErrorRecord::build(capture, &shared.ident, now, shared.next_seq());
    // `try_lock` only: the panic may have happened while this thread held a journal lock.
    let Some(mut journal) = try_lock(&shared.journal) else {
        let _ = storage::write_panic_file(&shared.dir, &record, now);
        shared.send(Message::Add(Box::new(record)));
        return;
    };
    journal.add(record.clone());
    let generation = shared.generation.fetch_add(1, Ordering::SeqCst) + 1;
    let bytes = journal.to_bytes();
    drop(journal);
    let saved = try_lock(&shared.written).is_some_and(|mut written| {
        let ok = storage::write_atomic(&shared.path(), &bytes).is_ok();
        if ok {
            *written = (*written).max(generation);
        }
        ok
    });
    if !saved {
        let _ = storage::write_panic_file(&shared.dir, &record, now);
    }
    shared.send(Message::Changed);
}

/// Routes `log::error!` (gpui reports its errors this way) into the journal; lower levels are
/// dropped. Returns false if another logger is already installed.
pub fn install_logger() -> bool {
    static LOGGER: JournalLogger = JournalLogger;
    let installed = log::set_logger(&LOGGER).is_ok();
    if installed {
        log::set_max_level(log::LevelFilter::Error);
    }
    installed
}

#[derive(Debug)]
struct JournalLogger;

impl log::Log for JournalLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() == log::Level::Error
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let location = match (record.file(), record.line()) {
            (Some(file), Some(line)) => Some(format!("{file}:{line}")),
            _ => None,
        };
        capture(
            Capture {
                kind: Some(Kind::Error),
                message: record.args().to_string(),
                location,
                thread: std::thread::current().name().map(str::to_string),
                context: Some(format!("log: {}", record.target())),
                backtrace: String::new(),
            },
            // `log` records carry no error value with a backtrace, and symbolizing one per call
            // would be costly if something logs an error every frame.
            false,
        );
    }

    fn flush(&self) {}
}

fn capture(capture: Capture, with_backtrace: bool) {
    let Some(shared) = SHARED.get() else { return };
    let backtrace = (with_backtrace && capture.backtrace.is_empty()).then(Backtrace::force_capture);
    shared.send(Message::Capture(capture, backtrace));
}

/// Records a handled error worth showing (a failed save, a plugin that did not load…), with
/// its chain and backtrace (anyhow's when captured, otherwise this call site's).
pub fn report(err: &anyhow::Error, context: &str) {
    let backtrace = err.backtrace();
    let backtrace = if backtrace.status() == BacktraceStatus::Captured {
        backtrace.to_string()
    } else {
        String::new()
    };
    capture(
        Capture {
            kind: Some(Kind::Error),
            message: format!("{err:#}"),
            location: None,
            thread: std::thread::current().name().map(str::to_string),
            context: Some(context.to_string()).filter(|c| !c.is_empty()),
            backtrace,
        },
        true,
    );
}

/// `report` for a plain message.
pub fn report_message(message: &str, context: &str) {
    report(&anyhow::anyhow!("{message}"), context);
}

/// Called from the worker thread on every change; keep it quick (e.g. push to a queue).
pub fn subscribe(listener: impl Fn(Event) + Send + Sync + 'static) {
    if let Some(shared) = SHARED.get() {
        lock(&shared.listeners).push(Box::new(listener));
    }
}

pub fn snapshot() -> Option<Snapshot> {
    let shared = SHARED.get()?;
    let journal = lock(&shared.journal);
    Some(Snapshot {
        records: journal.newest_first(),
        unseen: journal.unseen(),
        retention: journal.retention(),
        now: shared.clock.now(),
    })
}

pub fn unseen() -> usize {
    counts().0
}

/// (unseen, total) without cloning the records.
pub fn counts() -> (usize, usize) {
    SHARED.get().map_or((0, 0), |shared| {
        let journal = lock(&shared.journal);
        (journal.unseen(), journal.records().len())
    })
}

/// The user opened the errors view: hide the badge, keep the records.
pub fn mark_all_seen() {
    let Some(shared) = SHARED.get() else { return };
    if shared.mutate(Journal::mark_all_seen) {
        shared.send(Message::Changed);
    }
}

pub fn clear() {
    let Some(shared) = SHARED.get() else { return };
    shared.mutate(Journal::clear);
    shared.send(Message::Changed);
}

/// Applies a new retention on the worker; subscribers get `Event::Pruned`.
pub fn set_retention(retention: Retention) {
    if let Some(shared) = SHARED.get() {
        shared.send(Message::Retention(retention));
    }
}

/// Normal exit: records `clean_shutdown = true` synchronously.
pub fn shutdown() {
    let Some(shared) = SHARED.get() else { return };
    let now = shared.clock.now();
    shared.mutate(|journal| {
        journal.set_clean_shutdown(true);
        journal.prune(now, Prune::Periodic);
    });
    shared.persist();
}

/// Writes pending changes now (tests, diagnostics).
pub fn flush() {
    if let Some(shared) = SHARED.get() {
        shared.persist();
    }
}

pub fn data_dir() -> Option<&'static Path> {
    SHARED.get().map(|shared| shared.dir.as_path())
}

/// `macOS 26.1 (aarch64)`, read from SystemVersion.plist without running a process.
fn detect_os() -> String {
    let arch = std::env::consts::ARCH;
    let version = std::fs::read_to_string("/System/Library/CoreServices/SystemVersion.plist")
        .ok()
        .and_then(|plist| {
            let after = plist.split("<key>ProductVersion</key>").nth(1)?;
            let value = after.split("<string>").nth(1)?.split("</string>").next()?;
            Some(value.trim().to_string())
        });
    match version {
        Some(version) => format!("macOS {version} ({arch})"),
        None => format!("{} ({arch})", std::env::consts::OS),
    }
}
