//! Recently checked-out branches, read from the HEAD reflog (the "Recent" group of the
//! Branches popup).

use anyhow::Result;

use super::Repo;

/// How far back the reflog is scanned.
const REFLOG_DEPTH: &str = "-n500";

impl Repo {
    /// Branch names checked out most recently first (the current one included), at most `limit`.
    pub fn recent_branches(&self, limit: usize) -> Result<Vec<String>> {
        let out = self.run(&["reflog", "show", "--format=%gs", REFLOG_DEPTH, "HEAD", "--"])?;
        Ok(parse_recent(&out, limit))
    }
}

/// Collects `to` then `from` of each "checkout: moving from <from> to <to>" line, without
/// duplicates. Detached checkouts (hashes) are filtered later against the branch list.
fn parse_recent(reflog: &str, limit: usize) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for line in reflog.lines() {
        let Some(moves) = line.strip_prefix("checkout: moving from ") else {
            continue;
        };
        let Some((from, to)) = moves.split_once(" to ") else {
            continue;
        };
        for name in [to, from] {
            if names.len() < limit && !names.iter().any(|n| n == name) {
                names.push(name.to_string());
            }
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::parse_recent;
    use crate::testing::TempRepo;

    #[test]
    fn parses_checkouts_newest_first() {
        let reflog = "checkout: moving from main to feature\n\
                      commit: Add x\n\
                      checkout: moving from fix to main\n\
                      checkout: moving from main to fix\n";
        assert_eq!(parse_recent(reflog, 10), ["feature", "main", "fix"]);
        assert_eq!(parse_recent(reflog, 2), ["feature", "main"]);
    }

    #[test]
    fn reads_the_reflog_of_a_real_repo() {
        let repo = TempRepo::new();
        repo.commit_file("a.txt", "a", "init");
        repo.0.run(&["switch", "-q", "-c", "feature"]).unwrap();
        repo.0.run(&["switch", "-q", "main"]).unwrap();
        assert_eq!(repo.0.recent_branches(5).unwrap(), ["main", "feature"]);
    }
}
