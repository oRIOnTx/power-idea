//! The protocol-neutral file system the UI works with. Every method blocks: callers run
//! them on background threads, never on the UI thread.

use std::io::{Read, Write};

use crate::error::{ErrorKind, RemoteError, Result};
use crate::path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntryKind {
    File,
    Dir,
}

/// A remote file or folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    /// Absolute remote path.
    pub path: String,
    /// What it is, following symlinks (a link to a folder can be expanded).
    pub kind: EntryKind,
    pub symlink: bool,
    pub size: u64,
    /// Seconds since the Unix epoch.
    pub modified: Option<u64>,
    /// Permission bits (`0o755`), without the file type bits.
    pub permissions: Option<u32>,
    pub owner: Option<String>,
    pub group: Option<String>,
}

impl Entry {
    pub fn is_dir(&self) -> bool {
        self.kind == EntryKind::Dir
    }

    pub fn is_hidden(&self) -> bool {
        self.name.starts_with('.')
    }
}

/// Called with the bytes transferred so far; returning `false` cancels the transfer.
pub type Progress<'a> = &'a mut dyn FnMut(u64) -> bool;

/// One connected server, whatever the protocol.
pub trait RemoteFs: Send + Sync {
    /// The folder the server puts the user in (an absolute path).
    fn home(&self) -> Result<String>;
    /// Children of `dir`, without `.` and `..`.
    fn list(&self, dir: &str) -> Result<Vec<Entry>>;
    fn stat(&self, path: &str) -> Result<Entry>;
    /// Streams the file at `path` into `out`; returns the bytes copied.
    fn download(&self, path: &str, out: &mut dyn Write, progress: Progress<'_>) -> Result<u64>;
    /// Creates or replaces the file at `path` with everything from `input`.
    fn upload(&self, path: &str, input: &mut dyn Read, progress: Progress<'_>) -> Result<u64>;
    fn rename(&self, from: &str, to: &str) -> Result<()>;
    fn remove_file(&self, path: &str) -> Result<()>;
    /// Removes an empty folder.
    fn remove_dir(&self, path: &str) -> Result<()>;
    fn mkdir(&self, path: &str) -> Result<()>;
    fn chmod(&self, path: &str, mode: u32) -> Result<()>;
    /// False once the connection is known to be gone.
    fn is_alive(&self) -> bool;

    fn read(&self, path: &str) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        self.download(path, &mut bytes, &mut |_| true)?;
        Ok(bytes)
    }

    fn write(&self, path: &str, data: &[u8]) -> Result<()> {
        self.upload(path, &mut &data[..], &mut |_| true)?;
        Ok(())
    }

    /// Deletes a file, or a folder with everything in it.
    fn delete(&self, path: &str) -> Result<()> {
        if path::normalize("/", path) == "/" {
            return Err(RemoteError::other("Refusing to delete /"));
        }
        let entry = self.stat(path)?;
        // A symlink to a folder is removed as a link, never followed.
        if !entry.is_dir() || entry.symlink {
            return self.remove_file(path);
        }
        for child in self.list(path)? {
            self.delete(&child.path)?;
        }
        self.remove_dir(path)
    }

    /// Whether `path` exists.
    fn exists(&self, path: &str) -> Result<bool> {
        match self.stat(path) {
            Ok(_) => Ok(true),
            Err(err) if err.kind == ErrorKind::NotFound => Ok(false),
            Err(err) => Err(err),
        }
    }
}

/// Copies `input` to `out` in chunks, reporting progress and honouring cancellation.
pub(crate) fn copy_with_progress(
    input: &mut dyn Read,
    out: &mut dyn Write,
    progress: Progress<'_>,
) -> Result<u64> {
    let mut buf = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }
        out.write_all(buf.get(..n).unwrap_or_default())?;
        total += n as u64;
        if !progress(total) {
            return Err(RemoteError::cancelled());
        }
    }
    out.flush()?;
    Ok(total)
}

/// An in-memory server for tests of code built on [`RemoteFs`].
#[cfg(test)]
pub(crate) mod memory {
    use std::collections::BTreeMap;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    /// (contents — `None` for a folder, mtime, mode).
    type Node = (Option<Vec<u8>>, u64, u32);

    #[derive(Default)]
    pub(crate) struct MemoryFs {
        pub(crate) nodes: Mutex<BTreeMap<String, Node>>,
        pub(crate) dead: AtomicBool,
        pub(crate) clock: Mutex<u64>,
    }

    impl MemoryFs {
        pub(crate) fn new() -> Self {
            let fs = Self::default();
            fs.lock().insert("/".into(), (None, 0, 0o755));
            fs
        }

        fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Node>> {
            self.nodes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        }

        fn tick(&self) -> u64 {
            let mut clock = self
                .clock
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *clock += 1;
            *clock
        }

        pub(crate) fn put(&self, path: &str, text: &str) {
            let now = self.tick();
            self.lock()
                .insert(path.into(), (Some(text.as_bytes().to_vec()), now, 0o644));
        }

        pub(crate) fn dir(&self, path: &str) {
            let now = self.tick();
            self.lock().insert(path.into(), (None, now, 0o755));
        }

        fn check(&self) -> Result<()> {
            if self.dead.load(Ordering::Relaxed) {
                return Err(RemoteError::network("connection lost"));
            }
            Ok(())
        }

        fn entry(path: &str, node: &Node) -> Entry {
            Entry {
                name: path::file_name(path).to_string(),
                path: path.to_string(),
                kind: if node.0.is_some() {
                    EntryKind::File
                } else {
                    EntryKind::Dir
                },
                symlink: false,
                size: node.0.as_ref().map_or(0, |b| b.len() as u64),
                modified: Some(node.1),
                permissions: Some(node.2),
                owner: Some("me".into()),
                group: None,
            }
        }

        fn missing(path: &str) -> RemoteError {
            RemoteError::new(ErrorKind::NotFound, path.to_string())
        }
    }

    impl RemoteFs for MemoryFs {
        fn home(&self) -> Result<String> {
            self.check()?;
            Ok("/".into())
        }

        fn list(&self, dir: &str) -> Result<Vec<Entry>> {
            self.check()?;
            let nodes = self.lock();
            if !nodes.contains_key(dir) {
                return Err(Self::missing(dir));
            }
            Ok(nodes
                .iter()
                .filter(|(p, _)| p.as_str() != "/" && path::parent(p) == dir)
                .map(|(p, n)| Self::entry(p, n))
                .collect())
        }

        fn stat(&self, path: &str) -> Result<Entry> {
            self.check()?;
            let nodes = self.lock();
            nodes
                .get(path)
                .map(|n| Self::entry(path, n))
                .ok_or_else(|| Self::missing(path))
        }

        fn download(&self, path: &str, out: &mut dyn Write, progress: Progress<'_>) -> Result<u64> {
            self.check()?;
            let bytes = self
                .lock()
                .get(path)
                .and_then(|n| n.0.clone())
                .ok_or_else(|| Self::missing(path))?;
            copy_with_progress(&mut &bytes[..], out, progress)
        }

        fn upload(&self, path: &str, input: &mut dyn Read, progress: Progress<'_>) -> Result<u64> {
            self.check()?;
            if !self.lock().contains_key(&path::parent(path)) {
                return Err(Self::missing(&path::parent(path)));
            }
            let mut bytes = Vec::new();
            let n = copy_with_progress(input, &mut bytes, progress)?;
            let now = self.tick();
            self.lock().insert(path.into(), (Some(bytes), now, 0o644));
            Ok(n)
        }

        fn rename(&self, from: &str, to: &str) -> Result<()> {
            self.check()?;
            let mut nodes = self.lock();
            let moved: Vec<String> = nodes
                .keys()
                .filter(|p| path::is_within(from, p))
                .cloned()
                .collect();
            if moved.is_empty() {
                return Err(Self::missing(from));
            }
            for old in moved {
                if let Some(node) = nodes.remove(&old) {
                    nodes.insert(format!("{to}{}", &old[from.len()..]), node);
                }
            }
            Ok(())
        }

        fn remove_file(&self, path: &str) -> Result<()> {
            self.check()?;
            self.lock()
                .remove(path)
                .map(|_| ())
                .ok_or_else(|| Self::missing(path))
        }

        fn remove_dir(&self, path: &str) -> Result<()> {
            self.check()?;
            let mut nodes = self.lock();
            if nodes.keys().any(|p| p != path && path::is_within(path, p)) {
                return Err(RemoteError::other("directory not empty"));
            }
            nodes.remove(path).map(|_| ()).ok_or_else(|| Self::missing(path))
        }

        fn mkdir(&self, path: &str) -> Result<()> {
            self.check()?;
            self.dir(path);
            Ok(())
        }

        fn chmod(&self, path: &str, mode: u32) -> Result<()> {
            self.check()?;
            let mut nodes = self.lock();
            let node = nodes.get_mut(path).ok_or_else(|| Self::missing(path))?;
            node.2 = mode;
            Ok(())
        }

        fn is_alive(&self) -> bool {
            !self.dead.load(Ordering::Relaxed)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::memory::MemoryFs;
    use super::*;

    #[test]
    fn reads_writes_and_deletes_recursively() {
        let fs = MemoryFs::new();
        fs.dir("/srv");
        fs.dir("/srv/app");
        fs.put("/srv/app/a.txt", "a");
        fs.write("/srv/b.txt", b"bee").unwrap();
        assert_eq!(fs.read("/srv/b.txt").unwrap(), b"bee");
        fs.delete("/srv/app").unwrap();
        assert!(!fs.exists("/srv/app/a.txt").unwrap());
        assert!(!fs.exists("/srv/app").unwrap());
        assert!(fs.exists("/srv/b.txt").unwrap());
        assert!(fs.delete("/").is_err());
    }

    #[test]
    fn progress_can_cancel() {
        let data = vec![7u8; 200 * 1024];
        let mut out = Vec::new();
        let err = copy_with_progress(&mut &data[..], &mut out, &mut |done| done < 100 * 1024).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Cancelled);
    }
}
