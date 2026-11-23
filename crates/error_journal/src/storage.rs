//! Files on disk: `errors.json` (atomic writes) and `panic-<ts>.json` written by the panic hook
//! when the journal was busy, merged on the next start.

use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::journal::Loaded;
use crate::record::{ErrorRecord, JournalFile};

pub const FILE_NAME: &str = "errors.json";
const PANIC_PREFIX: &str = "panic-";

pub fn journal_path(dir: &Path) -> PathBuf {
    dir.join(FILE_NAME)
}

pub fn load(dir: &Path) -> Loaded {
    match fs::read(journal_path(dir)) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => Loaded::Missing,
        Err(err) => Loaded::Corrupt(err.to_string()),
        Ok(bytes) => match serde_json::from_slice::<JournalFile>(&bytes) {
            Ok(file) => Loaded::Valid(file),
            Err(err) => Loaded::Corrupt(err.to_string()),
        },
    }
}

/// Writes to a unique temp file next to `path`, then renames it over `path`.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_extension(format!("json.{}.{n}.tmp", std::process::id()));
    let result = (|| {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// One record the panic hook could not put into `errors.json`.
pub fn write_panic_file(dir: &Path, record: &ErrorRecord, now: u64) -> io::Result<()> {
    let bytes = serde_json::to_vec(record).map_err(io::Error::other)?;
    let name = format!("{PANIC_PREFIX}{now}-{}.json", record.id);
    write_atomic(&dir.join(name), &bytes)
}

/// Every panic file: the records that parse, and all paths (deleted once the merge is on disk).
pub fn panic_files(dir: &Path) -> (Vec<ErrorRecord>, Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return (Vec::new(), Vec::new());
    };
    let paths: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|path| {
            let is_json = path.extension().is_some_and(|ext| ext == "json");
            let is_panic = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(PANIC_PREFIX));
            is_json && is_panic
        })
        .collect();
    let records = paths
        .iter()
        .filter_map(|path| serde_json::from_slice::<ErrorRecord>(&fs::read(path).ok()?).ok())
        .collect();
    (records, paths)
}

pub fn remove_all(paths: &[PathBuf]) {
    for path in paths {
        let _ = fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::tests::{capture, ident};

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("error-journal-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn missing_corrupt_and_valid_files() {
        let dir = temp_dir("load");
        assert!(matches!(load(&dir), Loaded::Missing));
        fs::write(journal_path(&dir), b"{ not json").unwrap();
        assert!(matches!(load(&dir), Loaded::Corrupt(_)));
        let file = JournalFile::fresh(&ident());
        write_atomic(&journal_path(&dir), &file.to_bytes()).unwrap();
        assert!(matches!(load(&dir), Loaded::Valid(f) if f == file));
        let leftovers = fs::read_dir(&dir).unwrap().count();
        assert_eq!(leftovers, 1, "no temp files left behind");
    }

    #[test]
    fn panic_files_are_read_and_removed() {
        let dir = temp_dir("panic-files");
        let record = ErrorRecord::build(capture("boom"), &ident(), 5, 1);
        write_panic_file(&dir, &record, 5).unwrap();
        fs::write(dir.join("panic-bad.json"), b"garbage").unwrap();
        let (records, paths) = panic_files(&dir);
        assert_eq!((records, paths.len()), (vec![record], 2));
        remove_all(&paths);
        assert!(panic_files(&dir).1.is_empty());
    }
}
