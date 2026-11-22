//! Push, pull and fetch with a summary of what moved (commits, branches, files),
//! for notifications like "Pushed 3 commits to origin/main".

use std::collections::BTreeMap;

use anyhow::Result;

use super::Repo;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushSummary {
    UpToDate,
    Pushed {
        commits: u32,
        /// e.g. `origin/main`.
        upstream: String,
        /// The branch had no upstream before this push.
        new_branch: bool,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiffStat {
    pub files: u32,
    pub insertions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PullSummary {
    UpToDate,
    Updated {
        commits: u32,
        from: Option<String>,
        stat: DiffStat,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FetchSummary {
    /// New commits on remote branches that existed before.
    pub commits: u32,
    pub updated_branches: Vec<String>,
    pub new_branches: Vec<String>,
    pub deleted_branches: Vec<String>,
    /// Commits the current branch is behind its upstream afterwards.
    pub behind: u32,
}

impl PushSummary {
    pub fn message(&self) -> String {
        match self {
            Self::UpToDate => "Everything is up to date".into(),
            Self::Pushed {
                commits,
                upstream,
                new_branch: true,
            } => format!("Pushed new branch to {upstream} ({})", commits_text(*commits)),
            Self::Pushed {
                commits, upstream, ..
            } => {
                format!("Pushed {} to {upstream}", commits_text(*commits))
            }
        }
    }
}

impl PullSummary {
    pub fn message(&self) -> String {
        match self {
            Self::UpToDate => "Already up to date".into(),
            Self::Updated { commits, from, stat } => {
                let from = from.as_ref().map_or_default(|f| format!(" from {f}"));
                format!("{}{from} · {}", commits_text(*commits), stat.text())
            }
        }
    }
}

impl FetchSummary {
    pub fn message(&self) -> String {
        let mut parts = Vec::new();
        if self.commits > 0 {
            let branches = self.updated_branches.len();
            parts.push(format!(
                "{} in {branches} branch{}",
                commits_text(self.commits),
                if branches == 1 { "" } else { "es" }
            ));
        }
        if !self.new_branches.is_empty() {
            parts.push(format!("new: {}", self.new_branches.join(", ")));
        }
        if !self.deleted_branches.is_empty() {
            parts.push(format!("pruned: {}", self.deleted_branches.join(", ")));
        }
        if parts.is_empty() {
            parts.push("Nothing new".into());
        }
        if self.behind > 0 {
            parts.push(format!("current branch is {} behind", commits_text(self.behind)));
        }
        parts.join(" · ")
    }
}

impl DiffStat {
    fn text(&self) -> String {
        format!(
            "{} file{} changed (+{} −{})",
            self.files,
            if self.files == 1 { "" } else { "s" },
            self.insertions,
            self.deletions
        )
    }
}

fn commits_text(n: u32) -> String {
    format!("{n} commit{}", if n == 1 { "" } else { "s" })
}

/// Parses `git diff --shortstat`: ` 3 files changed, 10 insertions(+), 2 deletions(-)`.
fn parse_shortstat(text: &str) -> DiffStat {
    let mut stat = DiffStat::default();
    for part in text.split(',') {
        let mut words = part.split_whitespace();
        let (Some(n), Some(what)) = (words.next(), words.next()) else {
            continue;
        };
        let Ok(n) = n.parse() else { continue };
        if what.starts_with("file") {
            stat.files = n;
        } else if what.starts_with("insertion") {
            stat.insertions = n;
        } else if what.starts_with("deletion") {
            stat.deletions = n;
        }
    }
    stat
}

/// `refname objectname` lines → map.
fn parse_refs(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| line.split_once(' '))
        .filter(|(name, _)| !name.ends_with("/HEAD"))
        .map(|(name, hash)| (name.to_string(), hash.to_string()))
        .collect()
}

impl Repo {
    pub fn push_with_summary(&self) -> Result<PushSummary> {
        let before = self.status()?;
        let new_branch = before.upstream.is_none();
        let commits = if new_branch {
            // Commits no remote has yet.
            self.count(&["rev-list", "--count", "HEAD", "--not", "--remotes"])
        } else {
            before.ahead
        };
        if !new_branch && commits == 0 {
            return Ok(PushSummary::UpToDate);
        }
        self.push()?;
        let upstream = self.status()?.upstream.unwrap_or_else(|| "origin".into());
        Ok(PushSummary::Pushed {
            commits,
            upstream,
            new_branch,
        })
    }

    pub fn pull_with_summary(&self) -> Result<PullSummary> {
        let old = self.head();
        self.pull()?;
        let new = self.head();
        let (Some(old), Some(new)) = (old, new) else {
            return Ok(PullSummary::UpToDate);
        };
        if old == new {
            return Ok(PullSummary::UpToDate);
        }
        let range = format!("{old}..{new}");
        Ok(PullSummary::Updated {
            commits: self.count(&["rev-list", "--count", &range]),
            from: self.status()?.upstream,
            stat: parse_shortstat(&self.run(&["diff", "--shortstat", &old, &new]).unwrap_or_default()),
        })
    }

    pub fn fetch_with_summary(&self) -> Result<FetchSummary> {
        let before = self.remote_refs();
        self.fetch()?;
        let after = self.remote_refs();
        let short = |name: &str| name.trim_start_matches("refs/remotes/").to_string();
        let mut summary = FetchSummary::default();
        for (name, hash) in &after {
            match before.get(name) {
                None => summary.new_branches.push(short(name)),
                Some(old) if old != hash => {
                    summary.updated_branches.push(short(name));
                    summary.commits += self.count(&["rev-list", "--count", &format!("{old}..{hash}")]);
                }
                Some(_) => {}
            }
        }
        summary.deleted_branches = before
            .keys()
            .filter(|n| !after.contains_key(*n))
            .map(|n| short(n))
            .collect();
        summary.behind = self.status()?.behind;
        Ok(summary)
    }

    fn head(&self) -> Option<String> {
        self.run(&["rev-parse", "HEAD"])
            .ok()
            .map(|s| s.trim().to_string())
    }

    fn count(&self, args: &[&str]) -> u32 {
        self.run(args)
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0)
    }

    fn remote_refs(&self) -> BTreeMap<String, String> {
        parse_refs(
            &self
                .run(&[
                    "for-each-ref",
                    "--format=%(refname) %(objectname)",
                    "refs/remotes",
                ])
                .unwrap_or_default(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::remote_and_clone;

    #[test]
    fn parses_shortstat_and_refs() {
        assert_eq!(
            parse_shortstat(" 3 files changed, 10 insertions(+), 2 deletions(-)\n"),
            DiffStat {
                files: 3,
                insertions: 10,
                deletions: 2
            }
        );
        assert_eq!(
            parse_shortstat(" 1 file changed, 1 deletion(-)"),
            DiffStat {
                files: 1,
                insertions: 0,
                deletions: 1
            }
        );
        let refs = parse_refs("refs/remotes/origin/HEAD a\nrefs/remotes/origin/main b\n");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs["refs/remotes/origin/main"], "b");
    }

    #[test]
    fn formats_messages() {
        let push = PushSummary::Pushed {
            commits: 3,
            upstream: "origin/main".into(),
            new_branch: false,
        };
        assert_eq!(push.message(), "Pushed 3 commits to origin/main");
        let pull = PullSummary::Updated {
            commits: 1,
            from: Some("origin/main".into()),
            stat: DiffStat {
                files: 2,
                insertions: 5,
                deletions: 1,
            },
        };
        assert_eq!(
            pull.message(),
            "1 commit from origin/main · 2 files changed (+5 −1)"
        );
        let fetch = FetchSummary {
            commits: 4,
            updated_branches: vec!["origin/main".into()],
            new_branches: vec!["origin/feature".into()],
            behind: 4,
            ..Default::default()
        };
        assert_eq!(
            fetch.message(),
            "4 commits in 1 branch · new: origin/feature · current branch is 4 commits behind"
        );
        assert_eq!(FetchSummary::default().message(), "Nothing new");
    }

    #[test]
    fn summarizes_push_pull_and_fetch() {
        let (remote, local) = remote_and_clone();
        assert_eq!(local.0.push_with_summary().unwrap(), PushSummary::UpToDate);

        local.commit_file("a.txt", "1\n2\n", "second");
        local.commit_file("b.txt", "b\n", "third");
        assert_eq!(
            local.0.push_with_summary().unwrap(),
            PushSummary::Pushed {
                commits: 2,
                upstream: "origin/main".into(),
                new_branch: false
            }
        );

        remote.commit_file("c.txt", "c\nc\n", "from remote");
        let fetch = local.0.fetch_with_summary().unwrap();
        assert_eq!((fetch.commits, fetch.behind), (1, 1));
        assert_eq!(fetch.updated_branches, ["origin/main"]);

        assert_eq!(
            local.0.pull_with_summary().unwrap(),
            PullSummary::Updated {
                commits: 1,
                from: Some("origin/main".into()),
                stat: DiffStat {
                    files: 1,
                    insertions: 2,
                    deletions: 0
                }
            }
        );
        assert_eq!(local.0.pull_with_summary().unwrap(), PullSummary::UpToDate);

        local.git(&["checkout", "-q", "-b", "feature"]);
        local.commit_file("d.txt", "d\n", "feature work");
        assert_eq!(
            local.0.push_with_summary().unwrap(),
            PushSummary::Pushed {
                commits: 1,
                upstream: "origin/feature".into(),
                new_branch: true
            }
        );
    }
}
