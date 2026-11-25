//! `owner/repo` of a GitHub repository from a git remote URL.

use std::fmt;

/// A repository on github.com.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RepoId {
    pub owner: String,
    pub name: String,
}

impl fmt::Display for RepoId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

impl RepoId {
    pub fn html_url(&self) -> String {
        format!("https://github.com/{self}")
    }
}

const HOST: &str = "github.com";

/// Parses `git@github.com:o/r.git`, `ssh://git@github.com/o/r.git`, `https://github.com/o/r(.git)`
/// and friends; `None` for other hosts or anything that is not a two-segment repository path.
pub fn parse_github_remote(url: &str) -> Option<RepoId> {
    let url = url.trim();
    let path = if let Some((scheme, rest)) = url.split_once("://") {
        if !matches!(scheme, "https" | "http" | "ssh" | "git" | "git+ssh") {
            return None;
        }
        let (authority, path) = rest.split_once('/')?;
        let host = authority.rsplit('@').next()?;
        let host = host.split(':').next()?;
        if !host.eq_ignore_ascii_case(HOST) {
            return None;
        }
        path
    } else {
        // scp-like syntax: [user@]github.com:owner/repo
        let (authority, path) = url.split_once(':')?;
        let host = authority.rsplit('@').next()?;
        if !host.eq_ignore_ascii_case(HOST) {
            return None;
        }
        path
    };
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (owner, name) = path.split_once('/')?;
    let valid = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    (valid(owner) && valid(name) && name != "." && name != "..").then(|| RepoId {
        owner: owner.to_string(),
        name: name.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::unnecessary_wraps)] // compared against `parse_github_remote`'s Option
    fn id(o: &str, r: &str) -> Option<RepoId> {
        Some(RepoId {
            owner: o.into(),
            name: r.into(),
        })
    }

    #[test]
    fn parses_ssh_and_https_forms() {
        assert_eq!(parse_github_remote("git@github.com:o/r.git"), id("o", "r"));
        assert_eq!(parse_github_remote("git@github.com:o/r"), id("o", "r"));
        assert_eq!(parse_github_remote("https://github.com/o/r.git"), id("o", "r"));
        assert_eq!(parse_github_remote("https://github.com/o/r"), id("o", "r"));
        assert_eq!(parse_github_remote("https://github.com/o/r/"), id("o", "r"));
        assert_eq!(parse_github_remote("ssh://git@github.com/o/r.git"), id("o", "r"));
        assert_eq!(
            parse_github_remote("ssh://git@github.com:22/o/r.git"),
            id("o", "r")
        );
        assert_eq!(
            parse_github_remote("https://user@GitHub.com/My-Org/my.repo"),
            id("My-Org", "my.repo")
        );
        assert_eq!(
            parse_github_remote("  https://github.com/o/r.git\n"),
            id("o", "r")
        );
    }

    #[test]
    fn rejects_other_hosts_and_shapes() {
        assert_eq!(parse_github_remote("git@gitlab.com:o/r.git"), None);
        assert_eq!(parse_github_remote("https://github.example.com/o/r"), None);
        assert_eq!(parse_github_remote("https://github.com/o"), None);
        assert_eq!(parse_github_remote("https://github.com/o/r/tree/main"), None);
        assert_eq!(parse_github_remote("/srv/git/r.git"), None);
        assert_eq!(parse_github_remote("file:///github.com/o/r"), None);
        assert_eq!(parse_github_remote(""), None);
    }
}
