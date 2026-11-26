//! Has the server's copy changed since we opened it? Compared by size and
//! modification time before every upload of an edited remote file.

use crate::fs::Entry;

/// What we knew about a remote file when it was opened (or last uploaded).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snapshot {
    pub size: u64,
    pub modified: Option<u64>,
}

impl Snapshot {
    pub fn of(entry: &Entry) -> Self {
        Self {
            size: entry.size,
            modified: entry.modified,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conflict {
    /// Safe to upload.
    None,
    /// Someone changed the file on the server.
    Changed,
    /// The file is gone from the server.
    Deleted,
}

/// `opened`: the snapshot taken at open/last upload (`None` for a new file);
/// `current`: what the server reports now (`None` when the file does not exist).
pub fn check(opened: Option<Snapshot>, current: Option<Snapshot>) -> Conflict {
    match (opened, current) {
        (None, _) => Conflict::None,
        (Some(_), None) => Conflict::Deleted,
        (Some(then), Some(now)) => {
            // Servers without mtimes (rare FTP setups) fall back to the size alone.
            let time_changed = matches!((then.modified, now.modified), (Some(a), Some(b)) if a != b);
            if time_changed || then.size != now.size {
                Conflict::Changed
            } else {
                Conflict::None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::unnecessary_wraps)]
    fn snap(size: u64, modified: Option<u64>) -> Option<Snapshot> {
        Some(Snapshot { size, modified })
    }

    #[test]
    fn detects_changes() {
        assert_eq!(check(snap(10, Some(5)), snap(10, Some(5))), Conflict::None);
        assert_eq!(check(snap(10, Some(5)), snap(10, Some(6))), Conflict::Changed);
        assert_eq!(check(snap(10, Some(5)), snap(11, Some(5))), Conflict::Changed);
        assert_eq!(check(snap(10, None), snap(10, None)), Conflict::None);
        assert_eq!(check(snap(10, None), snap(12, None)), Conflict::Changed);
        assert_eq!(check(snap(10, Some(5)), None), Conflict::Deleted);
        assert_eq!(check(None, snap(1, Some(1))), Conflict::None);
    }
}
