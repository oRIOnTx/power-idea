//! The journal's rules as pure state: startup (version reset, crash detection), deduplication,
//! retention and the count / size limits. No I/O, no globals, no clock of its own.

use crate::record::{Capture, ErrorRecord, Identity, JournalFile, Kind};
use crate::retention::Retention;

/// Unique records kept at most.
pub const MAX_RECORDS: usize = 50;
/// Largest `errors.json` allowed.
pub const MAX_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_records: usize,
    pub max_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_records: MAX_RECORDS,
            max_bytes: MAX_BYTES,
        }
    }
}

/// What was on disk at startup.
#[derive(Debug)]
pub enum Loaded {
    Missing,
    /// Unreadable or invalid; the reason becomes one internal error record.
    Corrupt(String),
    Valid(JournalFile),
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct StartReport {
    pub version_reset: bool,
    pub corrupt: bool,
    pub previous_crashed: bool,
    pub pruned: usize,
}

/// Why pruning runs: "until restart" only drops records at startup or when just selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prune {
    Startup,
    Periodic,
    RetentionChanged,
}

#[derive(Debug)]
pub struct Journal {
    file: JournalFile,
    ident: Identity,
    limits: Limits,
    retention: Retention,
}

impl Journal {
    /// Builds this session's journal from what was on disk plus orphaned panic files.
    pub fn start(
        loaded: Loaded,
        orphans: Vec<ErrorRecord>,
        ident: Identity,
        retention: Retention,
        limits: Limits,
        now: u64,
    ) -> (Self, StartReport) {
        let mut report = StartReport::default();
        let mut corrupt = None;
        let mut file = match loaded {
            Loaded::Missing => JournalFile::fresh(&ident),
            Loaded::Corrupt(reason) => {
                report.corrupt = true;
                corrupt = Some(reason);
                JournalFile::fresh(&ident)
            }
            Loaded::Valid(file) if !same_build(&file.app_version, &file.build, &ident) => {
                report.version_reset = true;
                JournalFile::fresh(&ident)
            }
            Loaded::Valid(mut file) => {
                if !file.clean_shutdown {
                    report.previous_crashed = true;
                    for record in &mut file.records {
                        if record.session_id == file.session_id && record.kind == Kind::Panic {
                            record.from_crash = true;
                        }
                    }
                }
                file
            }
        };
        file.session_id.clone_from(&ident.session_id);
        file.clean_shutdown = false;
        let mut journal = Self {
            file,
            ident,
            limits,
            retention,
        };
        if let Some(reason) = corrupt {
            let capture = Capture {
                kind: Some(Kind::Error),
                message: format!("errors.json was unreadable and has been reset: {reason}"),
                context: Some("error journal".into()),
                ..Capture::default()
            };
            journal.add(ErrorRecord::build(capture, &journal.ident, now, 0));
        }
        for mut orphan in orphans {
            let known = journal.file.records.iter().any(|r| r.id == orphan.id);
            if known || !same_build(&orphan.app_version, &orphan.build, &journal.ident) {
                continue;
            }
            // Only a session that died before its next write leaves an orphan behind.
            orphan.from_crash = orphan.kind == Kind::Panic && orphan.session_id != journal.ident.session_id;
            journal.add(orphan);
        }
        report.pruned = journal.prune(now, Prune::Startup);
        journal.evict();
        (journal, report)
    }

    pub fn file(&self) -> &JournalFile {
        &self.file
    }

    pub fn identity(&self) -> &Identity {
        &self.ident
    }

    pub fn records(&self) -> &[ErrorRecord] {
        &self.file.records
    }

    /// Newest (`last_seen`) first.
    pub fn newest_first(&self) -> Vec<ErrorRecord> {
        let mut records = self.file.records.clone();
        records.sort_by(|a, b| b.last_seen.cmp(&a.last_seen).then_with(|| b.id.cmp(&a.id)));
        records
    }

    pub fn retention(&self) -> Retention {
        self.retention
    }

    /// Adds a record, or counts it against an existing one with the same fingerprint.
    pub fn add(&mut self, record: ErrorRecord) {
        let existing = self
            .file
            .records
            .iter_mut()
            .find(|r| r.fingerprint == record.fingerprint);
        match existing {
            Some(existing) => {
                existing.count = existing.count.saturating_add(record.count);
                existing.first_seen = existing.first_seen.min(record.first_seen);
                existing.last_seen = existing.last_seen.max(record.last_seen);
                existing.seen = existing.seen && record.seen;
                existing.from_crash |= record.from_crash;
                existing.session_id = record.session_id;
            }
            None => self.file.records.push(record),
        }
        self.evict();
    }

    /// Applies retention; returns how many records were removed.
    pub fn prune(&mut self, now: u64, when: Prune) -> usize {
        let before = self.file.records.len();
        match (self.retention.max_age(), when) {
            (Some(max_age), _) => self
                .file
                .records
                .retain(|r| r.last_seen.saturating_add(max_age) >= now),
            (None, Prune::Periodic) => {}
            (None, Prune::Startup | Prune::RetentionChanged) => {
                let session = &self.ident.session_id;
                // A crash must never disappear before the user has seen it.
                self.file
                    .records
                    .retain(|r| &r.session_id == session || (r.from_crash && !r.seen));
            }
        }
        before - self.file.records.len()
    }

    /// Switches retention and applies it at once; returns how many records were removed.
    pub fn set_retention(&mut self, retention: Retention, now: u64) -> usize {
        self.retention = retention;
        self.prune(now, Prune::RetentionChanged)
    }

    /// Returns whether anything changed.
    pub fn mark_all_seen(&mut self) -> bool {
        let mut changed = false;
        for record in self.file.records.iter_mut().filter(|r| !r.seen) {
            record.seen = true;
            changed = true;
        }
        changed
    }

    pub fn unseen(&self) -> usize {
        self.file.records.iter().filter(|r| !r.seen).count()
    }

    pub fn clear(&mut self) -> usize {
        std::mem::take(&mut self.file.records).len()
    }

    pub fn set_clean_shutdown(&mut self, clean: bool) {
        self.file.clean_shutdown = clean;
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.file.to_bytes()
    }

    /// Drops the least recently seen records until both limits hold.
    fn evict(&mut self) -> usize {
        let mut evicted = 0;
        while self.file.records.len() > self.limits.max_records
            || (!self.file.records.is_empty() && self.file.to_bytes().len() > self.limits.max_bytes)
        {
            let Some(oldest) = self
                .file
                .records
                .iter()
                .enumerate()
                .min_by_key(|(_, r)| (r.last_seen, r.first_seen))
                .map(|(ix, _)| ix)
            else {
                break;
            };
            self.file.records.remove(oldest);
            evicted += 1;
        }
        evicted
    }
}

fn same_build(app_version: &str, build: &str, ident: &Identity) -> bool {
    app_version == ident.app_version && build == ident.build
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::tests::{capture, ident};

    const DAY: u64 = 86_400;
    const NOW: u64 = 1_000 * DAY;

    fn record(message: &str, at: u64, seq: u64) -> ErrorRecord {
        ErrorRecord::build(capture(message), &ident(), at, seq)
    }

    fn previous(records: Vec<ErrorRecord>, clean: bool) -> Loaded {
        let mut records = records;
        for record in &mut records {
            record.session_id = "s1".into();
        }
        Loaded::Valid(JournalFile {
            app_version: "1.0.0".into(),
            build: "10".into(),
            session_id: "s1".into(),
            clean_shutdown: clean,
            records,
        })
    }

    fn start(loaded: Loaded, retention: Retention) -> (Journal, StartReport) {
        Journal::start(loaded, Vec::new(), ident(), retention, Limits::default(), NOW)
    }

    #[test]
    fn same_fingerprint_counts_instead_of_adding() {
        let (mut journal, _) = start(Loaded::Missing, Retention::Week);
        journal.add(record("index 3 out of range", NOW - 10, 1));
        journal.mark_all_seen();
        journal.add(record("index 9 out of range", NOW, 2));
        journal.add(record("something else", NOW, 3));
        assert_eq!(journal.records().len(), 2);
        let first = &journal.records()[0];
        assert_eq!(
            (first.count, first.first_seen, first.last_seen),
            (2, NOW - 10, NOW)
        );
        assert!(!first.seen, "a recurrence is news again");
    }

    #[test]
    fn evicts_oldest_beyond_the_count_limit() {
        let limits = Limits {
            max_records: 3,
            max_bytes: MAX_BYTES,
        };
        let (mut journal, _) = Journal::start(Loaded::Missing, vec![], ident(), Retention::Week, limits, NOW);
        for i in 0..5 {
            journal.add(record(&format!("error {}", "x".repeat(i as usize)), NOW + i, i));
        }
        let kept: Vec<u64> = journal.records().iter().map(|r| r.last_seen).collect();
        assert_eq!(kept, [NOW + 2, NOW + 3, NOW + 4]);
    }

    #[test]
    fn evicts_oldest_beyond_the_size_limit() {
        let limits = Limits {
            max_records: 50,
            max_bytes: 4 * 1024,
        };
        let (mut journal, _) = Journal::start(Loaded::Missing, vec![], ident(), Retention::Week, limits, NOW);
        for i in 0..20 {
            journal.add(record(
                &format!("error {} {}", "y".repeat(i as usize), "z".repeat(300)),
                NOW + i,
                i,
            ));
        }
        assert!(journal.to_bytes().len() <= 4 * 1024);
        assert!(journal.records().len() < 20);
        assert_eq!(journal.records().last().map(|r| r.last_seen), Some(NOW + 19));
    }

    #[test]
    fn a_new_version_starts_empty() {
        let mut loaded = previous(vec![record("old", NOW, 1)], true);
        if let Loaded::Valid(file) = &mut loaded {
            file.build = "9".into();
        }
        let (journal, report) = start(loaded, Retention::Month);
        assert!(report.version_reset);
        assert!(journal.records().is_empty());
        assert_eq!(journal.file().build, "10");
        assert!(!journal.file().clean_shutdown);
    }

    #[test]
    fn a_corrupt_file_leaves_one_internal_error() {
        let (journal, report) = start(Loaded::Corrupt("expected value".into()), Retention::Week);
        assert!(report.corrupt);
        assert_eq!(journal.records().len(), 1);
        assert!(
            journal.records()[0]
                .message
                .contains("errors.json was unreadable")
        );
    }

    #[test]
    fn seen_state_drives_the_badge() {
        let (mut journal, _) = start(Loaded::Missing, Retention::Week);
        assert_eq!(journal.unseen(), 0);
        journal.add(record("a", NOW, 1));
        journal.add(record("b", NOW, 2));
        assert_eq!(journal.unseen(), 2);
        assert!(journal.mark_all_seen());
        assert_eq!(journal.unseen(), 0);
        assert!(!journal.mark_all_seen());
        assert_eq!(journal.records().len(), 2, "seen records stay");
    }

    #[test]
    fn retention_prunes_by_last_seen() {
        let loaded = previous(
            vec![
                record("two days old", NOW - 2 * DAY, 1),
                record("an hour old", NOW - 3_600, 2),
            ],
            true,
        );
        let (mut journal, report) = start(loaded, Retention::Week);
        assert_eq!((report.pruned, journal.records().len()), (0, 2));
        assert_eq!(journal.set_retention(Retention::Day, NOW), 1);
        assert_eq!(journal.records()[0].message, "an hour old");
        assert_eq!(journal.prune(NOW + DAY + 1, Prune::Periodic), 1);
    }

    #[test]
    fn a_recurring_error_is_kept() {
        let (mut journal, _) = start(Loaded::Missing, Retention::Day);
        journal.add(record("flaky", NOW - 5 * DAY, 1));
        journal.add(record("flaky", NOW - 3_600, 2));
        assert_eq!(journal.prune(NOW, Prune::Periodic), 0);
        assert_eq!(journal.records()[0].count, 2);
    }

    fn panic_record(message: &str, seq: u64) -> ErrorRecord {
        let mut c = capture(message);
        c.kind = Some(Kind::Panic);
        ErrorRecord::build(c, &ident(), NOW - 60, seq)
    }

    #[test]
    fn until_restart_drops_the_previous_session_after_a_clean_exit() {
        let loaded = previous(vec![record("error", NOW - 60, 1), panic_record("panic", 2)], true);
        let (journal, report) = start(loaded, Retention::UntilRestart);
        assert!(!report.previous_crashed);
        assert_eq!(report.pruned, 2);
        assert!(journal.records().is_empty());
    }

    #[test]
    fn until_restart_keeps_an_unseen_crash_until_it_is_seen() {
        let loaded = previous(
            vec![record("error", NOW - 60, 1), panic_record("crash", 2)],
            false,
        );
        let (mut journal, report) = start(loaded, Retention::UntilRestart);
        assert!(report.previous_crashed);
        assert_eq!(journal.records().len(), 1);
        assert!(journal.records()[0].from_crash);
        // Not seen this session (and the session ends cleanly): still there next time.
        assert_eq!(journal.prune(NOW, Prune::Periodic), 0);
        journal.set_clean_shutdown(true);
        let mut next_ident = ident();
        next_ident.session_id = "s3".into();
        let (journal, _) = Journal::start(
            Loaded::Valid(journal.file().clone()),
            vec![],
            next_ident.clone(),
            Retention::UntilRestart,
            Limits::default(),
            NOW,
        );
        assert_eq!(journal.records().len(), 1);
        // Seen once: gone at the following start, not while the user is looking at it.
        let mut journal = journal;
        journal.mark_all_seen();
        assert_eq!(journal.prune(NOW, Prune::Periodic), 0);
        let mut last_ident = next_ident;
        last_ident.session_id = "s4".into();
        let (journal, _) = Journal::start(
            Loaded::Valid(journal.file().clone()),
            vec![],
            last_ident,
            Retention::UntilRestart,
            Limits::default(),
            NOW,
        );
        assert!(journal.records().is_empty());
    }

    #[test]
    fn orphaned_panic_files_merge_once() {
        let orphan = {
            let mut r = panic_record("died", 7);
            r.session_id = "s1".into();
            r.id = "s1-7".into();
            r
        };
        let (journal, _) = Journal::start(
            previous(vec![], false),
            vec![orphan.clone(), orphan],
            ident(),
            Retention::Week,
            Limits::default(),
            NOW,
        );
        assert_eq!(journal.records().len(), 1);
        assert!(journal.records()[0].from_crash);
    }
}
