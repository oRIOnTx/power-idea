//! Throwaway git repositories for tests.

use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::Repo;

pub(crate) struct TempRepo(pub(crate) Repo);

impl TempRepo {
    pub(crate) fn new() -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "power-idea-vcs-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        let repo = Repo {
            root: dir.canonicalize().unwrap(),
        };
        repo.run(&["init", "-q", "-b", "main"]).unwrap();
        repo.run(&["config", "user.name", "Test"]).unwrap();
        repo.run(&["config", "user.email", "test@example.com"]).unwrap();
        repo.run(&["config", "commit.gpgsign", "false"]).unwrap();
        Self(repo)
    }

    pub(crate) fn write(&self, path: &str, text: &str) {
        let path = self.0.root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    pub(crate) fn read(&self, path: &str) -> String {
        fs::read_to_string(self.0.root.join(path)).unwrap()
    }

    /// Writes `path` and commits it; returns the new commit hash.
    pub(crate) fn commit_file(&self, path: &str, text: &str, message: &str) -> String {
        self.write(path, text);
        self.0.commit(message, &[path.to_string()], false).unwrap();
        self.git(&["rev-parse", "HEAD"]).trim().to_string()
    }

    pub(crate) fn git(&self, args: &[&str]) -> String {
        self.0.run(args).unwrap()
    }
}

/// `remote` (non-bare, accepts pushes) and a clone of it.
pub(crate) fn remote_and_clone() -> (TempRepo, TempRepo) {
    let remote = TempRepo::new();
    remote.commit_file("a.txt", "1\n", "first");
    remote.git(&["config", "receive.denyCurrentBranch", "updateInstead"]);
    let local = TempRepo::new();
    fs::remove_dir_all(local.0.root.join(".git")).unwrap();
    let url = remote.0.root.to_string_lossy().into_owned();
    local.git(&["clone", "-q", &url, "."]);
    local.git(&["config", "user.name", "Test"]);
    local.git(&["config", "user.email", "test@example.com"]);
    local.git(&["config", "commit.gpgsign", "false"]);
    (remote, local)
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0.root);
    }
}
