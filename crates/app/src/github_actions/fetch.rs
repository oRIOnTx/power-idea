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
    /// Runs started from the IDE that are on no listed page (errors are skipped).
    pub tracked: Vec<Option<Run>>,
}

pub fn snapshot(
    client: &GitHub,
    root: &Path,
    want_all: bool,
    jobs_of: Option<u64>,
    tracked: &[u64],
) -> Snapshot {
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
    let tracked = tracked
        .iter()
        .map(|id| client.run(*id).ok().map(|f| f.value))
        .collect();
    Snapshot {
        branch,
        head_sha,
        branch_runs,
        all_runs,
        jobs,
        tracked,
    }
}

/// The project's workflows and what `workflow_dispatch` each accepts: from the checked-out file,
/// else from the default branch on GitHub.
pub fn workflows(client: &GitHub, root: &Path) -> ApiResult<Vec<super::WorkflowEntry>> {
    let workflows = client.workflows()?.value;
    Ok(workflows
        .into_iter()
        .filter(|w| w.state == "active")
        .map(|workflow| {
            let local = std::fs::read_to_string(root.join(&workflow.path)).ok();
            // Dynamic workflows (Pages, Dependabot) have no file to read.
            let in_repo = workflow.path.starts_with(".github/workflows/");
            let yaml = local.or_else(|| in_repo.then(|| client.file(&workflow.path, "").ok()).flatten());
            let dispatch = yaml.as_deref().and_then(ci::dispatch::dispatch_inputs);
            super::WorkflowEntry { workflow, dispatch }
        })
        .collect())
}

/// A client for the project's GitHub repository, or why there is none (for MCP tools when the
/// tab was never opened). Blocks: run it off the UI thread.
pub fn connect(root: &Path) -> Result<GitHub, String> {
    match check_setup(root) {
        SetupResult::Connected { client, .. } => Ok(client),
        SetupResult::Unavailable(setup) => Err(match setup {
            Setup::NoRepo => "the project is not a git repository".into(),
            Setup::NoRemote => "the repository has no remote".into(),
            Setup::NotGitHub(url) => format!("the remote {url} is not on GitHub"),
            Setup::NotConnected { .. } => {
                "not connected to GitHub: run `gh auth login` or save a token in the IDE's GitHub Actions tab (⌘8)".into()
            }
            Setup::Checking | Setup::Ready => "GitHub is not available".into(),
        }),
    }
}
