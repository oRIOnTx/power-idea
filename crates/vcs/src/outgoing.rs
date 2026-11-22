//! What a push would send: the target remote and branch, the commits it doesn't have yet,
//! and the files they touch (for the Push dialog).

use anyhow::{Result, bail};

use super::history::parse_name_status;
use super::{Commit, CommitFile, PushSummary, Repo};

/// Hash of the empty tree: the base to diff a root commit against.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbabb8a7";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushTarget {
    pub remote: String,
    /// The remote's push URL, to tell remotes apart.
    pub url: Option<String>,
    pub branch: String,
    /// The remote has no such branch yet.
    pub new_branch: bool,
}

impl PushTarget {
    /// e.g. `origin/main`.
    pub fn name(&self) -> String {
        format!("{}/{}", self.remote, self.branch)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outgoing {
    pub local: String,
    /// `None` when the repository has no remote to push to.
    pub target: Option<PushTarget>,
    /// Newest first.
    pub commits: Vec<Commit>,
    /// Everything the commits change, taken together.
    pub files: Vec<CommitFile>,
    /// Commits on the target the local branch lacks: the push would be rejected.
    pub behind: u32,
}

impl Repo {
    /// What pushing the local branch `branch` would send, and where.
    pub fn outgoing(&self, branch: &str) -> Result<Outgoing> {
        let Some(target) = self.push_target(branch)? else {
            return Ok(Outgoing {
                local: branch.into(),
                target: None,
                commits: self.commits(&[branch])?,
                files: Vec::new(),
                behind: 0,
            });
        };
        let remote_ref = format!("refs/remotes/{}", target.name());
        let (commits, files, behind) = if target.new_branch {
            let commits = self.commits(&[branch, "--not", "--remotes"])?;
            let files = match commits.last() {
                Some(oldest) => self.changed_files(&self.parent_or_empty(&oldest.hash), branch)?,
                None => Vec::new(),
            };
            (commits, files, 0)
        } else {
            let range = format!("{remote_ref}..{branch}");
            let files = self.run(&["diff", "--name-status", "-M", &format!("{remote_ref}...{branch}")])?;
            let behind = self
                .run(&["rev-list", "--count", &format!("{branch}..{remote_ref}")])?
                .trim()
                .parse()
                .unwrap_or(0);
            (self.commits(&[&range])?, parse_name_status(&files), behind)
        };
        Ok(Outgoing {
            local: branch.into(),
            target: Some(target),
            commits,
            files,
            behind,
        })
    }

    /// Pushes `local` to `target` and makes it the branch's upstream.
    pub fn push_to(&self, local: &str, target: &PushTarget, tags: bool) -> Result<PushSummary> {
        let commits = self.outgoing(local)?.commits.len() as u32;
        if commits == 0 && !target.new_branch && !tags {
            return Ok(PushSummary::UpToDate);
        }
        let refspec = format!("refs/heads/{local}:refs/heads/{}", target.branch);
        let mut args = vec!["push", "--set-upstream"];
        if tags {
            args.push("--tags");
        }
        args.extend([target.remote.as_str(), refspec.as_str()]);
        self.run(&args)?;
        Ok(PushSummary::Pushed {
            commits,
            upstream: target.name(),
            new_branch: target.new_branch,
        })
    }

    /// The configured upstream, else the same-named branch on the default remote.
    fn push_target(&self, branch: &str) -> Result<Option<PushTarget>> {
        if !self.has_ref(&format!("refs/heads/{branch}")) {
            bail!("{branch} is not a local branch");
        }
        let config = |key: &str| {
            self.run(&["config", "--get", &format!("branch.{branch}.{key}")])
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        };
        let upstream = config("remote")
            .filter(|remote| remote != ".")
            .zip(config("merge").and_then(|m| m.strip_prefix("refs/heads/").map(str::to_string)));
        let (remote, remote_branch) = match upstream {
            Some(upstream) => upstream,
            None => {
                let remotes = self.remotes()?;
                let remote = remotes
                    .iter()
                    .find(|r| *r == "origin")
                    .or(remotes.first())
                    .cloned();
                let Some(remote) = remote else {
                    return Ok(None);
                };
                (remote, branch.to_string())
            }
        };
        let new_branch = !self.has_ref(&format!("refs/remotes/{remote}/{remote_branch}"));
        let url = self
            .run(&["remote", "get-url", "--push", &remote])
            .ok()
            .map(|s| s.trim().to_string());
        Ok(Some(PushTarget {
            remote,
            url,
            branch: remote_branch,
            new_branch,
        }))
    }

    fn has_ref(&self, name: &str) -> bool {
        self.run(&["rev-parse", "--verify", "--quiet", name]).is_ok()
    }

    fn parent_or_empty(&self, hash: &str) -> String {
        let parent = format!("{hash}^");
        if self.has_ref(&parent) {
            parent
        } else {
            EMPTY_TREE.into()
        }
    }

    fn changed_files(&self, from: &str, to: &str) -> Result<Vec<CommitFile>> {
        Ok(parse_name_status(&self.run(&[
            "diff",
            "--name-status",
            "-M",
            from,
            to,
        ])?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::remote_and_clone;

    fn subjects(outgoing: &Outgoing) -> Vec<&str> {
        outgoing.commits.iter().map(|c| c.subject.as_str()).collect()
    }

    fn paths(outgoing: &Outgoing) -> Vec<&str> {
        outgoing.files.iter().map(|f| f.path.as_str()).collect()
    }

    #[test]
    fn lists_commits_and_files_for_the_upstream() {
        let (remote, local) = remote_and_clone();
        let outgoing = local.0.outgoing("main").unwrap();
        assert!(outgoing.commits.is_empty() && outgoing.files.is_empty());
        let target = outgoing.target.unwrap();
        assert_eq!((target.name(), target.new_branch), ("origin/main".into(), false));
        assert_eq!(target.url.as_deref(), Some(remote.0.root.to_str().unwrap()));

        local.commit_file("a.txt", "1\n2\n", "second");
        local.commit_file("dir/b.txt", "b\n", "third");
        let outgoing = local.0.outgoing("main").unwrap();
        assert_eq!(subjects(&outgoing), ["third", "second"]);
        assert_eq!(paths(&outgoing), ["a.txt", "dir/b.txt"]);
        assert_eq!(outgoing.behind, 0);

        remote.commit_file("c.txt", "c\n", "from remote");
        local.git(&["fetch", "-q"]);
        assert_eq!(local.0.outgoing("main").unwrap().behind, 1);
    }

    #[test]
    fn pushes_a_new_branch_and_tracks_it() {
        let (remote, local) = remote_and_clone();
        local.git(&["checkout", "-q", "-b", "feature"]);
        local.commit_file("d.txt", "d\n", "feature work");
        let outgoing = local.0.outgoing("feature").unwrap();
        let target = outgoing.target.clone().unwrap();
        assert_eq!(
            (target.name(), target.new_branch),
            ("origin/feature".into(), true)
        );
        assert_eq!(
            (subjects(&outgoing), paths(&outgoing)),
            (vec!["feature work"], vec!["d.txt"])
        );

        assert_eq!(
            local.0.push_to("feature", &target, false).unwrap(),
            PushSummary::Pushed {
                commits: 1,
                upstream: "origin/feature".into(),
                new_branch: true
            }
        );
        assert!(remote.git(&["branch", "--list", "feature"]).contains("feature"));
        let after = local.0.outgoing("feature").unwrap();
        let target = after.target.unwrap();
        assert!(after.commits.is_empty() && !target.new_branch);
        assert_eq!(
            local.0.push_to("feature", &target, false).unwrap(),
            PushSummary::UpToDate
        );
    }

    #[test]
    fn without_a_remote_there_is_no_target() {
        let (_remote, local) = remote_and_clone();
        local.git(&["remote", "remove", "origin"]);
        let outgoing = local.0.outgoing("main").unwrap();
        assert_eq!(outgoing.target, None);
        assert_eq!(subjects(&outgoing), ["first"]);
    }
}
