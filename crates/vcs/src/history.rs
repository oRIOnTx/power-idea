//! Git log for the Log tool window: commits with parents and ref labels,
//! commit details, and repository operations (cherry-pick, rebase, …).

use anyhow::{Result, bail};

use super::Repo;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    Head,
    Local,
    Remote,
    Tag,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefLabel {
    pub name: String,
    pub kind: RefKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub hash: String,
    pub short_hash: String,
    pub parents: Vec<String>,
    pub author: String,
    pub email: String,
    /// Unix seconds (author date).
    pub timestamp: i64,
    pub refs: Vec<RefLabel>,
    pub subject: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitFile {
    pub path: String,
    pub orig_path: Option<String>,
    /// Status letter from `--name-status` (A, M, D, R, C, T).
    pub status: char,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitDetails {
    pub hash: String,
    pub author: String,
    pub email: String,
    pub timestamp: i64,
    pub message: String,
    pub files: Vec<CommitFile>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetMode {
    Soft,
    Mixed,
    Hard,
}

/// A multi-step operation waiting for the user (after conflicts).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Merge,
    Rebase,
    CherryPick,
    Revert,
}

impl Operation {
    pub fn label(self) -> &'static str {
        match self {
            Self::Merge => "Merge",
            Self::Rebase => "Rebase",
            Self::CherryPick => "Cherry-pick",
            Self::Revert => "Revert",
        }
    }

    fn command(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Rebase => "rebase",
            Self::CherryPick => "cherry-pick",
            Self::Revert => "revert",
        }
    }
}

/// Parses `%D` decorations such as `HEAD -> main, origin/main, tag: v1`.
fn parse_refs(decorations: &str, remotes: &[String]) -> Vec<RefLabel> {
    let mut refs = Vec::new();
    for part in decorations.split(", ").map(str::trim).filter(|p| !p.is_empty()) {
        if let Some(branch) = part.strip_prefix("HEAD -> ") {
            refs.push(RefLabel {
                name: "HEAD".into(),
                kind: RefKind::Head,
            });
            refs.push(RefLabel {
                name: branch.into(),
                kind: RefKind::Local,
            });
        } else if part == "HEAD" {
            refs.push(RefLabel {
                name: "HEAD".into(),
                kind: RefKind::Head,
            });
        } else if let Some(tag) = part.strip_prefix("tag: ") {
            refs.push(RefLabel {
                name: tag.into(),
                kind: RefKind::Tag,
            });
        } else if part.ends_with("/HEAD") {
            continue;
        } else {
            let is_remote = remotes.iter().any(|r| part.starts_with(&format!("{r}/")));
            refs.push(RefLabel {
                name: part.into(),
                kind: if is_remote {
                    RefKind::Remote
                } else {
                    RefKind::Local
                },
            });
        }
    }
    refs
}

fn parse_log(out: &str, remotes: &[String]) -> Vec<LogEntry> {
    out.split('\x1e')
        .filter_map(|record| {
            let mut f = record.trim_start_matches('\n').split('\x1f');
            let hash = f.next().filter(|h| !h.is_empty())?.to_string();
            Some(LogEntry {
                hash,
                short_hash: f.next()?.into(),
                parents: f.next()?.split_whitespace().map(str::to_string).collect(),
                author: f.next()?.into(),
                email: f.next()?.into(),
                timestamp: f.next()?.parse().unwrap_or(0),
                refs: parse_refs(f.next()?, remotes),
                subject: f.next()?.into(),
            })
        })
        .collect()
}

fn parse_name_status(out: &str) -> Vec<CommitFile> {
    out.lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let status = parts.next()?.chars().next()?;
            let first = parts.next()?.to_string();
            Some(match parts.next() {
                Some(second) => CommitFile {
                    path: second.into(),
                    orig_path: Some(first),
                    status,
                },
                None => CommitFile {
                    path: first,
                    orig_path: None,
                    status,
                },
            })
        })
        .collect()
}

impl Repo {
    pub fn remotes(&self) -> Result<Vec<String>> {
        Ok(self.run(&["remote"])?.lines().map(str::to_string).collect())
    }

    /// Up to `limit` commits of `revision` (all refs when `None`), newest first.
    pub fn history(&self, revision: Option<&str>, limit: usize) -> Result<Vec<LogEntry>> {
        if !self.has_head() {
            return Ok(Vec::new());
        }
        let limit = format!("-n{limit}");
        let format = "--format=%H%x1f%h%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%D%x1f%s%x1e";
        let mut args = vec!["log", "--date-order", "--decorate=short", &limit, format];
        match revision {
            Some(rev) => args.extend([rev, "--"]),
            None => args.extend(["--branches", "--remotes", "--tags", "HEAD", "--"]),
        }
        Ok(parse_log(&self.run(&args)?, &self.remotes()?))
    }

    pub fn commit_details(&self, hash: &str) -> Result<CommitDetails> {
        let header = self.run(&["show", "-s", "--format=%H%x1f%an%x1f%ae%x1f%at%x1f%B", hash])?;
        let mut f = header.splitn(5, '\x1f');
        let (Some(full), Some(author), Some(email), Some(time), Some(message)) =
            (f.next(), f.next(), f.next(), f.next(), f.next())
        else {
            bail!("unexpected git show output");
        };
        // Merge commits are compared with their first parent, like JetBrains does.
        let files = self.run(&[
            "diff-tree",
            "-r",
            "-M",
            "--name-status",
            "--no-commit-id",
            "--root",
            "-m",
            "--first-parent",
            hash,
        ])?;
        Ok(CommitDetails {
            hash: full.into(),
            author: author.into(),
            email: email.into(),
            timestamp: time.trim().parse().unwrap_or(0),
            message: message.trim_end().into(),
            files: parse_name_status(&files),
        })
    }

    /// Patch of one file in `hash` (against its first parent).
    pub fn commit_file_diff(&self, hash: &str, path: &str) -> Result<String> {
        self.run(&[
            "show",
            "--no-color",
            "--no-ext-diff",
            "-M",
            "-m",
            "--first-parent",
            "--format=",
            hash,
            "--",
            path,
        ])
    }

    pub fn cherry_pick(&self, hash: &str) -> Result<String> {
        self.run(&["cherry-pick", hash])
    }

    pub fn revert(&self, hash: &str) -> Result<String> {
        self.run(&["revert", "--no-edit", hash])
    }

    pub fn reset(&self, hash: &str, mode: ResetMode) -> Result<String> {
        let mode = match mode {
            ResetMode::Soft => "--soft",
            ResetMode::Mixed => "--mixed",
            ResetMode::Hard => "--hard",
        };
        self.run(&["reset", mode, hash])
    }

    /// Checks out a commit (detached HEAD).
    pub fn checkout_revision(&self, hash: &str) -> Result<String> {
        self.run(&["switch", "--detach", hash])
    }

    pub fn create_branch_at(&self, name: &str, start: &str, checkout: bool) -> Result<String> {
        if checkout {
            self.run(&["switch", "-c", name, start])
        } else {
            self.run(&["branch", name, start])
        }
    }

    pub fn create_tag(&self, name: &str, hash: &str) -> Result<String> {
        self.run(&["tag", name, hash])
    }

    /// Merges `branch` into the current branch.
    pub fn merge(&self, branch: &str) -> Result<String> {
        self.run(&["merge", "--no-edit", branch])
    }

    /// Rebases the current branch onto `onto`.
    pub fn rebase(&self, onto: &str) -> Result<String> {
        self.run(&["rebase", onto])
    }

    pub fn rename_branch(&self, old: &str, new: &str) -> Result<String> {
        self.run(&["branch", "-m", old, new])
    }

    /// Deletes a local branch; `force` also deletes unmerged ones.
    pub fn delete_branch(&self, name: &str, force: bool) -> Result<String> {
        self.run(&["branch", if force { "-D" } else { "-d" }, name])
    }

    /// Deletes `remote/branch` on the server.
    pub fn delete_remote_branch(&self, remote_branch: &str) -> Result<String> {
        let Some((remote, branch)) = remote_branch.split_once('/') else {
            bail!("{remote_branch} is not a remote branch");
        };
        self.run(&["push", remote, "--delete", branch])
    }

    /// Pushes a local branch, setting its upstream on first push.
    pub fn push_branch(&self, name: &str) -> Result<String> {
        self.run(&["push", "--set-upstream", "origin", name])
    }

    /// The merge/rebase/cherry-pick/revert waiting for conflict resolution.
    pub fn operation_in_progress(&self) -> Option<Operation> {
        let git_dir = self.run(&["rev-parse", "--absolute-git-dir"]).ok()?;
        let git_dir = std::path::Path::new(git_dir.trim());
        if git_dir.join("rebase-merge").exists() || git_dir.join("rebase-apply").exists() {
            Some(Operation::Rebase)
        } else if git_dir.join("MERGE_HEAD").exists() {
            Some(Operation::Merge)
        } else if git_dir.join("CHERRY_PICK_HEAD").exists() {
            Some(Operation::CherryPick)
        } else if git_dir.join("REVERT_HEAD").exists() {
            Some(Operation::Revert)
        } else {
            None
        }
    }

    pub fn continue_operation(&self, operation: Operation) -> Result<String> {
        self.run(&["-c", "core.editor=true", operation.command(), "--continue"])
    }

    pub fn abort_operation(&self, operation: Operation) -> Result<String> {
        self.run(&[operation.command(), "--abort"])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_decorations() {
        let remotes = vec!["origin".to_string()];
        let refs = parse_refs(
            "HEAD -> main, origin/main, origin/HEAD, tag: v1.0, feature/x",
            &remotes,
        );
        let kinds: Vec<_> = refs.iter().map(|r| (r.name.as_str(), r.kind)).collect();
        assert_eq!(
            kinds,
            [
                ("HEAD", RefKind::Head),
                ("main", RefKind::Local),
                ("origin/main", RefKind::Remote),
                ("v1.0", RefKind::Tag),
                ("feature/x", RefKind::Local),
            ]
        );
    }

    #[test]
    fn parses_name_status_with_renames() {
        let files = parse_name_status("M\tsrc/a.rs\nR100\told.rs\tnew.rs\nA\tb.rs\n");
        assert_eq!(
            files[0],
            CommitFile {
                path: "src/a.rs".into(),
                orig_path: None,
                status: 'M'
            }
        );
        assert_eq!(
            files[1],
            CommitFile {
                path: "new.rs".into(),
                orig_path: Some("old.rs".into()),
                status: 'R'
            }
        );
        assert_eq!(files[2].status, 'A');
    }
}
