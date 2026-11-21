//! Git integration through the system `git` CLI.
//!
//! Using the CLI (like JetBrains IDEs do) means the user's credentials, SSH
//! agent, hooks and `.gitconfig` all work without extra configuration.

mod graph;
mod history;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use anyhow::{Context as _, Result, bail};

pub use graph::{Anchor, GraphRow, Segment, layout as graph_layout};
pub use history::{CommitDetails, CommitFile, LogEntry, Operation, RefKind, RefLabel, ResetMode};

#[derive(Debug, Clone)]
pub struct Repo {
    /// Repository work tree root.
    root: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChangeKind {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    Conflicted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    /// Path relative to the repository root.
    pub path: String,
    /// Previous path for renames and copies.
    pub orig_path: Option<String>,
    pub kind: ChangeKind,
    /// Whether the change (or part of it) is in the index.
    pub staged: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
    /// Current branch, or `None` when HEAD is detached.
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub changes: Vec<FileChange>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub hash: String,
    pub short_hash: String,
    pub author: String,
    pub date: String,
    pub subject: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Branch {
    pub name: String,
    pub is_remote: bool,
    pub is_current: bool,
}

impl Repo {
    /// Finds the repository containing `path`, if any.
    pub fn discover(path: &Path) -> Option<Self> {
        let output = git_command(path)
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let root = String::from_utf8(output.stdout).ok()?;
        Some(Self {
            root: PathBuf::from(root.trim_end()),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn run(&self, args: &[&str]) -> Result<String> {
        let output = git_command(&self.root)
            .args(args)
            .output()
            .with_context(|| format!("running git {}", args.join(" ")))?;
        check(output, args)
    }

    pub fn status(&self) -> Result<Status> {
        let out = self.run(&[
            "status",
            "--porcelain=v2",
            "--branch",
            "-z",
            "--untracked-files=all",
        ])?;
        Ok(parse_status(&out))
    }

    /// Unified diff of `path` between HEAD and the working tree.
    pub fn diff_file(&self, change: &FileChange) -> Result<String> {
        if change.kind == ChangeKind::Untracked {
            return self.diff_untracked(&change.path);
        }
        let mut args = vec!["diff", "--no-color", "--no-ext-diff", "-M"];
        if self.has_head() {
            args.push("HEAD");
        } else {
            args.push("--cached");
        }
        args.push("--");
        if let Some(orig) = &change.orig_path {
            args.push(orig);
        }
        args.push(&change.path);
        self.run(&args)
    }

    fn diff_untracked(&self, path: &str) -> Result<String> {
        let args = [
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--no-index",
            "--",
            "/dev/null",
            path,
        ];
        let output = git_command(&self.root).args(args).output()?;
        // `--no-index` exits with 1 when the files differ.
        if output.status.code() == Some(1) {
            return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
        }
        check(output, &args)
    }

    fn has_head(&self) -> bool {
        self.run(&["rev-parse", "--verify", "--quiet", "HEAD"]).is_ok()
    }

    /// Commits exactly `paths` (staging them first), leaving other changes alone.
    pub fn commit(&self, message: &str, paths: &[String], amend: bool) -> Result<String> {
        if message.trim().is_empty() && !amend {
            bail!("commit message is empty");
        }
        if paths.is_empty() && !amend {
            bail!("no files selected");
        }
        if !paths.is_empty() {
            let mut add = vec!["add", "-A", "--"];
            add.extend(paths.iter().map(String::as_str));
            self.run(&add)?;
        }
        let mut args = vec!["commit", "--cleanup=strip"];
        if amend {
            args.push("--amend");
        }
        if message.trim().is_empty() {
            args.push("--no-edit");
        } else {
            args.extend(["-m", message]);
        }
        if !paths.is_empty() {
            args.push("--");
            args.extend(paths.iter().map(String::as_str));
        }
        self.run(&args)
    }

    /// Message of the last commit, for amending.
    pub fn last_commit_message(&self) -> Result<String> {
        self.run(&["log", "-1", "--format=%B"])
            .map(|s| s.trim_end().to_string())
    }

    pub fn push(&self) -> Result<String> {
        let status = self.status()?;
        if status.upstream.is_some() {
            self.run(&["push"])
        } else {
            self.run(&["push", "--set-upstream", "origin", "HEAD"])
        }
    }

    pub fn pull(&self) -> Result<String> {
        self.run(&["pull", "--no-edit"])
    }

    pub fn fetch(&self) -> Result<String> {
        self.run(&["fetch", "--all", "--prune"])
    }

    /// Discards working tree and index changes of `change` (deletes untracked files).
    pub fn rollback(&self, change: &FileChange) -> Result<()> {
        match change.kind {
            ChangeKind::Untracked => {
                std::fs::remove_file(self.root.join(&change.path))?;
            }
            ChangeKind::Added => {
                self.run(&["rm", "--cached", "--force", "--quiet", "--", &change.path])?;
                std::fs::remove_file(self.root.join(&change.path))?;
            }
            _ => {
                let mut args = vec!["restore", "--source=HEAD", "--staged", "--worktree", "--"];
                if let Some(orig) = &change.orig_path {
                    args.push(orig);
                }
                args.push(&change.path);
                self.run(&args)?;
                if change.kind == ChangeKind::Renamed {
                    let _ = std::fs::remove_file(self.root.join(&change.path));
                }
            }
        }
        Ok(())
    }

    pub fn log(&self, limit: usize) -> Result<Vec<Commit>> {
        if !self.has_head() {
            return Ok(Vec::new());
        }
        let limit = format!("-n{limit}");
        let out = self.run(&[
            "log",
            &limit,
            "--date=short",
            "--format=%H%x1f%h%x1f%an%x1f%ad%x1f%s%x1e",
        ])?;
        Ok(out
            .split('\x1e')
            .filter_map(|record| {
                let mut f = record.trim_start_matches('\n').split('\x1f');
                Some(Commit {
                    hash: f.next().filter(|h| !h.is_empty())?.to_string(),
                    short_hash: f.next()?.to_string(),
                    author: f.next()?.to_string(),
                    date: f.next()?.to_string(),
                    subject: f.next()?.to_string(),
                })
            })
            .collect())
    }

    /// Patch introduced by `commit`.
    pub fn show_commit(&self, commit: &str) -> Result<String> {
        self.run(&["show", "--no-color", "--no-ext-diff", "-M", "--format=", commit])
    }

    pub fn branches(&self) -> Result<Vec<Branch>> {
        let out = self.run(&[
            "for-each-ref",
            "--format=%(refname)%09%(HEAD)",
            "refs/heads",
            "refs/remotes",
        ])?;
        Ok(out
            .lines()
            .filter_map(|line| {
                let (refname, head) = line.split_once('\t')?;
                let (name, is_remote) = match refname.strip_prefix("refs/heads/") {
                    Some(name) => (name, false),
                    None => (refname.strip_prefix("refs/remotes/")?, true),
                };
                if is_remote && name.ends_with("/HEAD") {
                    return None;
                }
                Some(Branch {
                    name: name.to_string(),
                    is_remote,
                    is_current: head == "*",
                })
            })
            .collect())
    }

    /// Switches to `branch`; a remote branch gets a local tracking branch.
    pub fn checkout(&self, branch: &Branch) -> Result<String> {
        if branch.is_remote {
            self.run(&["switch", "--track", &branch.name])
        } else {
            self.run(&["switch", &branch.name])
        }
    }

    pub fn create_branch(&self, name: &str) -> Result<String> {
        self.run(&["switch", "-c", name])
    }
}

fn git_command(dir: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(dir)
        .args(["-c", "core.quotepath=false", "-c", "color.ui=false"])
        // Never block on an interactive prompt or editor.
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        // Background status refreshes must not fight with the user's git commands.
        .env("GIT_OPTIONAL_LOCKS", "0");
    command
}

fn check(output: Output, args: &[&str]) -> Result<String> {
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let message = if stderr.trim().is_empty() {
        stdout.trim()
    } else {
        stderr.trim()
    };
    bail!(
        "git {} failed: {message}",
        args.first().copied().unwrap_or_default()
    )
}

fn parse_status(out: &str) -> Status {
    let mut status = Status::default();
    let mut fields = out.split('\0');
    while let Some(entry) = fields.next() {
        if let Some(header) = entry.strip_prefix("# ") {
            let (key, value) = header.split_once(' ').unwrap_or((header, ""));
            match key {
                "branch.head" if value != "(detached)" => status.branch = Some(value.to_string()),
                "branch.upstream" => status.upstream = Some(value.to_string()),
                "branch.ab" => {
                    for part in value.split(' ') {
                        if let Some(n) = part.strip_prefix('+') {
                            status.ahead = n.parse().unwrap_or(0);
                        } else if let Some(n) = part.strip_prefix('-') {
                            status.behind = n.parse().unwrap_or(0);
                        }
                    }
                }
                _ => {}
            }
            continue;
        }

        let change = match entry.as_bytes().first() {
            Some(b'?') => FileChange {
                path: entry[2..].to_string(),
                orig_path: None,
                kind: ChangeKind::Untracked,
                staged: false,
            },
            Some(b'1') => {
                let parts: Vec<&str> = entry.splitn(9, ' ').collect();
                let Some((xy, path)) = parts.get(1).zip(parts.get(8)) else {
                    continue;
                };
                ordinary_change(xy, path, None)
            }
            Some(b'2') => {
                let parts: Vec<&str> = entry.splitn(10, ' ').collect();
                let orig = fields.next().map(str::to_string);
                let Some((xy, path)) = parts.get(1).zip(parts.get(9)) else {
                    continue;
                };
                ordinary_change(xy, path, orig)
            }
            Some(b'u') => {
                let parts: Vec<&str> = entry.splitn(11, ' ').collect();
                let Some(path) = parts.get(10) else { continue };
                FileChange {
                    path: path.to_string(),
                    orig_path: None,
                    kind: ChangeKind::Conflicted,
                    staged: false,
                }
            }
            _ => continue,
        };
        status.changes.push(change);
    }
    status.changes.sort_by(|a, b| a.path.cmp(&b.path));
    status
}

fn ordinary_change(xy: &str, path: &str, orig_path: Option<String>) -> FileChange {
    let mut chars = xy.chars();
    let (x, y) = (chars.next().unwrap_or('.'), chars.next().unwrap_or('.'));
    let kind = match (x, y) {
        ('R' | 'C', _) | (_, 'R' | 'C') => ChangeKind::Renamed,
        ('A', _) => ChangeKind::Added,
        ('D', _) | (_, 'D') => ChangeKind::Deleted,
        _ => ChangeKind::Modified,
    };
    FileChange {
        path: path.to_string(),
        orig_path,
        kind,
        staged: x != '.',
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TempRepo(Repo);

    impl TempRepo {
        fn new() -> Self {
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

        fn write(&self, path: &str, text: &str) {
            let path = self.0.root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
    }

    impl Drop for TempRepo {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0.root);
        }
    }

    fn kinds(status: &Status) -> Vec<(&str, ChangeKind, bool)> {
        status
            .changes
            .iter()
            .map(|c| (c.path.as_str(), c.kind, c.staged))
            .collect()
    }

    #[test]
    fn parses_porcelain_v2() {
        let out = "# branch.oid abc\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -1\0\
            1 .M N... 100644 100644 100644 aaa aaa src/a.rs\0\
            1 A. N... 000000 100644 100644 000 bbb new file.rs\0\
            2 R. N... 100644 100644 100644 ccc ccc R100 b2.rs\0b.rs\0\
            ? notes.txt\0";
        let status = parse_status(out);
        assert_eq!(status.branch.as_deref(), Some("main"));
        assert_eq!((status.ahead, status.behind), (2, 1));
        assert_eq!(
            kinds(&status),
            [
                ("b2.rs", ChangeKind::Renamed, true),
                ("new file.rs", ChangeKind::Added, true),
                ("notes.txt", ChangeKind::Untracked, false),
                ("src/a.rs", ChangeKind::Modified, false),
            ]
        );
        assert_eq!(status.changes[0].orig_path.as_deref(), Some("b.rs"));
    }

    #[test]
    fn status_diff_commit_log_roundtrip() {
        let repo = TempRepo::new();
        repo.write("a.txt", "one\n");
        let status = repo.0.status().unwrap();
        assert_eq!(kinds(&status), [("a.txt", ChangeKind::Untracked, false)]);
        assert!(repo.0.diff_file(&status.changes[0]).unwrap().contains("+one"));

        repo.0.commit("initial", &["a.txt".into()], false).unwrap();
        assert!(repo.0.status().unwrap().changes.is_empty());

        repo.write("a.txt", "two\n");
        repo.write("b.txt", "other\n");
        let status = repo.0.status().unwrap();
        let a = status.changes.iter().find(|c| c.path == "a.txt").unwrap();
        let diff = repo.0.diff_file(a).unwrap();
        assert!(diff.contains("-one") && diff.contains("+two"));

        // Committing only a.txt leaves b.txt untouched.
        repo.0.commit("second", &["a.txt".into()], false).unwrap();
        assert_eq!(
            kinds(&repo.0.status().unwrap()),
            [("b.txt", ChangeKind::Untracked, false)]
        );

        let log = repo.0.log(10).unwrap();
        assert_eq!(
            log.iter().map(|c| c.subject.as_str()).collect::<Vec<_>>(),
            ["second", "initial"]
        );
        assert!(repo.0.show_commit(&log[0].hash).unwrap().contains("+two"));
    }

    #[test]
    fn rollback_and_branches() {
        let repo = TempRepo::new();
        repo.write("a.txt", "one\n");
        repo.0.commit("initial", &["a.txt".into()], false).unwrap();

        repo.write("a.txt", "changed\n");
        repo.write("new.txt", "x\n");
        for change in repo.0.status().unwrap().changes {
            repo.0.rollback(&change).unwrap();
        }
        assert!(repo.0.status().unwrap().changes.is_empty());
        assert_eq!(fs::read_to_string(repo.0.root.join("a.txt")).unwrap(), "one\n");

        repo.0.create_branch("feature").unwrap();
        let branches = repo.0.branches().unwrap();
        let current: Vec<_> = branches
            .iter()
            .filter(|b| b.is_current)
            .map(|b| b.name.as_str())
            .collect();
        assert_eq!(current, ["feature"]);
        let main = branches.iter().find(|b| b.name == "main").unwrap();
        repo.0.checkout(main).unwrap();
        assert_eq!(repo.0.status().unwrap().branch.as_deref(), Some("main"));
    }
}
