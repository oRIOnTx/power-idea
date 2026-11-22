//! Branch tools beyond checkout/merge: worktrees, tracking and comparing with the
//! working tree.

use std::path::Path;

use anyhow::Result;

use super::Repo;

impl Repo {
    /// Adds a worktree at `path` on `start`; with `new_branch` it starts a branch there
    /// (a branch can be checked out in only one worktree).
    pub fn add_worktree(&self, path: &Path, start: &str, new_branch: Option<&str>) -> Result<String> {
        let path = path.to_string_lossy();
        let mut args = vec!["worktree", "add"];
        if let Some(name) = new_branch {
            args.extend(["-b", name]);
        }
        args.extend([path.as_ref(), start]);
        self.run(&args)
    }

    /// Sets (or with `None` removes) the remote branch `local` tracks.
    pub fn set_upstream(&self, local: &str, upstream: Option<&str>) -> Result<String> {
        match upstream {
            Some(upstream) => self.run(&["branch", &format!("--set-upstream-to={upstream}"), local]),
            None => self.run(&["branch", "--unset-upstream", local]),
        }
    }

    /// Patch from `revision` to the working tree (staged and unstaged changes alike).
    pub fn diff_with_working_tree(&self, revision: &str) -> Result<String> {
        self.run(&["diff", "--no-color", "--no-ext-diff", "-M", revision, "--"])
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::testing::{TempRepo, remote_and_clone};

    #[test]
    fn adds_worktrees_with_a_new_or_free_branch() {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "1\n", "first");
        repo.git(&["branch", "free"]);
        let dir = repo.0.root().with_extension("wt");
        repo.0
            .add_worktree(&dir.join("new"), "main", Some("topic"))
            .unwrap();
        assert!(dir.join("new/a.txt").exists());
        repo.0.add_worktree(&dir.join("free"), "free", None).unwrap();
        assert!(
            repo.0.add_worktree(&dir.join("again"), "main", None).is_err(),
            "main is checked out"
        );
        let list = repo.git(&["worktree", "list"]);
        assert!(list.contains("[topic]") && list.contains("[free]"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn sets_and_unsets_the_tracked_branch() {
        let (_remote, local) = remote_and_clone();
        local.git(&["branch", "topic"]);
        local.0.set_upstream("topic", Some("origin/main")).unwrap();
        let upstream = || {
            local
                .0
                .branches()
                .unwrap()
                .into_iter()
                .find(|b| b.name == "topic")
                .unwrap()
                .upstream
        };
        assert_eq!(upstream().as_deref(), Some("origin/main"));
        local.0.set_upstream("topic", None).unwrap();
        assert_eq!(upstream(), None);
    }

    #[test]
    fn diffs_a_revision_with_the_working_tree() {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "1\n", "first");
        repo.write("a.txt", "1\n2\n");
        let patch = repo.0.diff_with_working_tree("main").unwrap();
        assert!(patch.contains("+2"));
        assert!(repo.0.diff_with_working_tree("main~5").is_err());
    }
}
