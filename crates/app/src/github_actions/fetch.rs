//! Blocking work of the GitHub Actions tab, run on background threads: finding the
//! repository and token, and one poll's worth of requests.

use std::path::Path;
use std::sync::Arc;

use ci::github::{ApiResult, Fetched};
use ci::transport::HttpTransport;
use ci::{GitHub, Job, RepoId, Run, RunFilter, TokenSource};

/// Why the tab cannot show runs, or that it can.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Setup {
    Checking,
    NoRepo,
    NoRemote,
    /// The remotes point elsewhere; the URL of the first one.
    NotGitHub(String),
    NotConnected {
        gh_installed: bool,
        error: Option<String>,
    },
    Ready,
}

impl Setup {
    pub fn gh_installed(&self) -> bool {
        matches!(
            self,
            Self::NotConnected {
                gh_installed: true,
                ..
            }
        )
    }
}

pub enum SetupResult {
    Connected { client: GitHub, source: TokenSource },
    Unavailable(Setup),
}

pub fn check_setup(root: &Path) -> SetupResult {
    let Some(repo) = vcs::Repo::discover(root) else {
        return SetupResult::Unavailable(Setup::NoRepo);
    };
    let repo_id = match github_repo(&repo) {
        Ok(id) => id,
        Err(setup) => return SetupResult::Unavailable(setup),
    };
    match ci::auth::resolve_token() {
        Some((token, source)) => SetupResult::Connected {
            client: GitHub::new(Arc::new(HttpTransport::default()), token, repo_id),
            source,
        },
        None => SetupResult::Unavailable(Setup::NotConnected {
            gh_installed: ci::auth::gh_installed(),
            error: None,
        }),
    }
}

/// `origin` when it is on GitHub, else the first other remote that is.
fn github_repo(repo: &vcs::Repo) -> Result<RepoId, Setup> {
    let mut names = repo.remotes().unwrap_or_default();
    if let Some(ix) = names.iter().position(|n| n == "origin") {
        names.swap(0, ix);
    }
    let urls: Vec<String> = names.iter().filter_map(|n| repo.remote_url(n)).collect();
    if let Some(id) = urls.iter().find_map(|u| ci::parse_github_remote(u)) {
        return Ok(id);
    }
    Err(urls.into_iter().next().map_or(Setup::NoRemote, Setup::NotGitHub))
}

/// One poll: where HEAD is, and the runs (and jobs) the tab shows.
pub struct Snapshot {
    pub branch: Option<String>,
    pub head_sha: Option<String>,
    /// `None` when HEAD is detached.
    pub branch_runs: Option<ApiResult<Fetched<Vec<Run>>>>,
    pub all_runs: Option<ApiResult<Fetched<Vec<Run>>>>,
    pub jobs: Option<(u64, ApiResult<Fetched<Vec<Job>>>)>,
}

pub fn snapshot(client: &GitHub, root: &Path, want_all: bool, jobs_of: Option<u64>) -> Snapshot {
    let repo = vcs::Repo::discover(root);
    let branch = repo.as_ref().and_then(vcs::Repo::current_branch);
    let head_sha = repo.as_ref().and_then(vcs::Repo::head_sha);
    let branch_runs = branch.as_ref().map(|branch| {
        client.runs(&RunFilter {
            branch: Some(branch.clone()),
            ..RunFilter::default()
        })
    });
    let all_runs = want_all.then(|| client.runs(&RunFilter::default()));
    let jobs = jobs_of.map(|run_id| (run_id, client.jobs(run_id)));
    Snapshot {
        branch,
        head_sha,
        branch_runs,
        all_runs,
        jobs,
    }
}
