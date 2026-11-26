//! Uploads and downloads of files and whole folders: planning the steps, running them
//! with progress and cancellation, and measuring speed.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::error::{RemoteError, Result};
use crate::fs::{Entry, RemoteFs};
use crate::path;
use crate::session::Connection;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Upload,
    Download,
}

/// One file to copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub local: PathBuf,
    pub remote: String,
    pub size: u64,
}

/// Folders to create (in order) and files to copy.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    pub dirs: Vec<(PathBuf, String)>,
    pub files: Vec<Step>,
}

impl Plan {
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|s| s.size).sum()
    }
}

/// Progress of a running transfer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
    pub done: u64,
    pub total: u64,
    pub files_done: usize,
    pub files_total: usize,
    /// The file being copied (remote path).
    pub current: String,
}

/// A remote name that is safe as one local path component (a hostile server could
/// send `..` or names with slashes).
pub fn safe_local_name(name: &str) -> Option<&str> {
    let bad = name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', '\0']);
    (!bad).then_some(name)
}

/// Uploading `local` (a file or a folder) into the remote folder `remote_dir`.
pub fn plan_upload(local: &Path, remote_dir: &str) -> Result<Plan> {
    let name = local
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| RemoteError::other(format!("{} has no usable name", local.display())))?;
    let mut plan = Plan::default();
    add_local(local, &path::join(remote_dir, name), &mut plan)?;
    Ok(plan)
}

fn add_local(local: &Path, remote: &str, plan: &mut Plan) -> Result<()> {
    let meta = fs::symlink_metadata(local)?;
    // Symlinks are skipped: following them can loop or upload far more than meant.
    if meta.file_type().is_symlink() {
        return Ok(());
    }
    if meta.is_dir() {
        plan.dirs.push((local.to_path_buf(), remote.to_string()));
        let mut children: Vec<_> = fs::read_dir(local)?.filter_map(std::result::Result::ok).collect();
        children.sort_by_key(fs::DirEntry::file_name);
        for child in children {
            let Some(name) = child.file_name().to_str().map(str::to_string) else {
                continue;
            };
            add_local(&child.path(), &path::join(remote, &name), plan)?;
        }
    } else {
        plan.files.push(Step {
            local: local.to_path_buf(),
            remote: remote.to_string(),
            size: meta.len(),
        });
    }
    Ok(())
}

/// Downloading `entry` (a file or a folder) into the local folder `local_dir`.
pub fn plan_download(fs: &dyn RemoteFs, entry: &Entry, local_dir: &Path) -> Result<Plan> {
    let mut plan = Plan::default();
    add_remote(fs, entry, local_dir, &mut plan)?;
    Ok(plan)
}

fn add_remote(fs: &dyn RemoteFs, entry: &Entry, local_dir: &Path, plan: &mut Plan) -> Result<()> {
    let Some(name) = safe_local_name(&entry.name) else {
        return Ok(());
    };
    let local = local_dir.join(name);
    if entry.is_dir() {
        if entry.symlink {
            return Ok(());
        }
        plan.dirs.push((local.clone(), entry.path.clone()));
        for child in fs.list(&entry.path)? {
            add_remote(fs, &child, &local, plan)?;
        }
    } else {
        plan.files.push(Step {
            local,
            remote: entry.path.clone(),
            size: entry.size,
        });
    }
    Ok(())
}

/// Runs `plan`; `report` gets progress (throttled by the caller if needed).
/// A cancelled transfer stops after the current chunk and leaves no partial download.
pub fn execute(
    conn: &Connection,
    direction: Direction,
    plan: &Plan,
    cancel: &AtomicBool,
    report: &mut dyn FnMut(&Status),
) -> Result<u64> {
    let mut status = Status {
        total: plan.total_bytes(),
        files_total: plan.files.len(),
        ..Status::default()
    };
    for (local, remote) in &plan.dirs {
        match direction {
            Direction::Upload => {
                conn.run(|fs| {
                    if fs.exists(remote)? {
                        Ok(())
                    } else {
                        fs.mkdir(remote)
                    }
                })?;
            }
            Direction::Download => fs::create_dir_all(local)?,
        }
    }
    for step in &plan.files {
        if cancel.load(Ordering::Relaxed) {
            return Err(RemoteError::cancelled());
        }
        status.current.clone_from(&step.remote);
        report(&status);
        let base = status.done;
        let mut on_progress = |n: u64| {
            status.done = base + n;
            report(&status);
            !cancel.load(Ordering::Relaxed)
        };
        match direction {
            Direction::Upload => upload_file(conn, step, &mut on_progress)?,
            Direction::Download => download_file(conn, step, &mut on_progress)?,
        }
        status.done = base + step.size;
        status.files_done += 1;
        report(&status);
    }
    Ok(status.done)
}

fn upload_file(conn: &Connection, step: &Step, progress: &mut dyn FnMut(u64) -> bool) -> Result<()> {
    let progress = std::cell::RefCell::new(progress);
    conn.run(|fs| {
        let mut file = fs::File::open(&step.local)?;
        let mut progress = progress.borrow_mut();
        fs.upload(&step.remote, &mut file, &mut |n| (*progress)(n))
    })?;
    Ok(())
}

fn download_file(conn: &Connection, step: &Step, progress: &mut dyn FnMut(u64) -> bool) -> Result<()> {
    let part = part_path(&step.local);
    let progress = std::cell::RefCell::new(progress);
    let result = conn.run(|fs| {
        let mut file = fs::File::create(&part)?;
        let mut progress = progress.borrow_mut();
        fs.download(&step.remote, &mut file, &mut |n| (*progress)(n))
    });
    match result {
        Ok(_) => {
            fs::rename(&part, &step.local)?;
            Ok(())
        }
        Err(err) => {
            fs::remove_file(&part).ok();
            Err(err)
        }
    }
}

/// `name.ext` → `name.ext.part`, the download's temporary name.
fn part_path(local: &Path) -> PathBuf {
    let mut name = local
        .file_name()
        .map_or_else(Default::default, std::ffi::OsStr::to_os_string);
    name.push(".part");
    local.with_file_name(name)
}

/// Transfer speed over the last few seconds.
#[derive(Debug, Default)]
pub struct SpeedMeter {
    samples: std::collections::VecDeque<(Instant, u64)>,
}

impl SpeedMeter {
    const WINDOW: Duration = Duration::from_secs(3);

    pub fn record(&mut self, at: Instant, done: u64) {
        self.samples.push_back((at, done));
        while self
            .samples
            .front()
            .is_some_and(|(t, _)| at.duration_since(*t) > Self::WINDOW)
            && self.samples.len() > 2
        {
            self.samples.pop_front();
        }
    }

    /// Bytes per second, or `None` before there is enough to measure.
    pub fn bytes_per_sec(&self) -> Option<u64> {
        let (first, last) = (self.samples.front()?, self.samples.back()?);
        let secs = last.0.duration_since(first.0).as_secs_f64();
        (secs > 0.2).then(|| (last.1.saturating_sub(first.1) as f64 / secs) as u64)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::fs::memory::MemoryFs;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("power-idea-transfer-{}-{name}", std::process::id()));
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn connection(fs: Arc<MemoryFs>) -> Connection {
        Connection::open(Box::new(move || Ok(fs.clone() as Arc<dyn RemoteFs>))).unwrap()
    }

    #[test]
    fn uploads_a_folder_recursively() {
        let dir = temp_dir("up");
        fs::create_dir_all(dir.join("site/css")).unwrap();
        fs::write(dir.join("site/index.html"), "<h1>").unwrap();
        fs::write(dir.join("site/css/a.css"), "a{}").unwrap();
        let plan = plan_upload(&dir.join("site"), "/var/www").unwrap();
        assert_eq!(plan.dirs.len(), 2);
        assert_eq!(plan.files.len(), 2);
        assert_eq!(plan.total_bytes(), 7);

        let remote = Arc::new(MemoryFs::new());
        remote.dir("/var");
        remote.dir("/var/www");
        let conn = connection(remote.clone());
        let mut last = Status::default();
        let done = execute(
            &conn,
            Direction::Upload,
            &plan,
            &AtomicBool::new(false),
            &mut |s| {
                last = s.clone();
            },
        )
        .unwrap();
        assert_eq!(done, 7);
        assert_eq!(last.files_done, 2);
        assert_eq!(remote.read("/var/www/site/css/a.css").unwrap(), b"a{}");
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn downloads_a_folder_and_skips_unsafe_names() {
        let remote = Arc::new(MemoryFs::new());
        remote.dir("/logs");
        remote.put("/logs/app.log", "line\n");
        remote.dir("/logs/old");
        remote.put("/logs/old/1.log", "x");
        let dir = temp_dir("down");
        let root = remote.stat("/logs").unwrap();
        let plan = plan_download(&*remote, &root, &dir).unwrap();
        let conn = connection(remote);
        execute(
            &conn,
            Direction::Download,
            &plan,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(fs::read_to_string(dir.join("logs/app.log")).unwrap(), "line\n");
        assert_eq!(fs::read_to_string(dir.join("logs/old/1.log")).unwrap(), "x");
        assert!(!dir.join("logs/app.log.part").exists());
        assert_eq!(safe_local_name(".."), None);
        assert_eq!(safe_local_name("a/b"), None);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn cancelling_leaves_no_partial_file() {
        let remote = Arc::new(MemoryFs::new());
        remote.put("/big.bin", &"x".repeat(300 * 1024));
        let dir = temp_dir("cancel");
        let plan = plan_download(&*remote, &remote.stat("/big.bin").unwrap(), &dir).unwrap();
        let conn = connection(remote);
        let cancel = AtomicBool::new(false);
        let err = execute(&conn, Direction::Download, &plan, &cancel, &mut |s| {
            if s.done > 0 {
                cancel.store(true, Ordering::Relaxed);
            }
        })
        .unwrap_err();
        assert_eq!(err.kind, crate::error::ErrorKind::Cancelled);
        assert!(!dir.join("big.bin").exists());
        assert!(!dir.join("big.bin.part").exists());
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn measures_speed() {
        let mut meter = SpeedMeter::default();
        let start = Instant::now();
        assert_eq!(meter.bytes_per_sec(), None);
        meter.record(start, 0);
        meter.record(start + Duration::from_secs(1), 1000);
        assert_eq!(meter.bytes_per_sec(), Some(1000));
    }
}
