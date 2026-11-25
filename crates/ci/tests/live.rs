//! Against the real api.github.com: `cargo test -p ci -- --ignored`.
//! Uses `gh auth token` when gh is logged in, else anonymous requests (60 an hour).

use std::sync::Arc;

use ci::github::RunFilter;
use ci::transport::{HttpTransport, Request, Transport as _};
use ci::{GitHub, RepoId};

const REPO: (&str, &str) = ("rust-lang", "cargo");

#[test]
#[ignore = "network: talks to api.github.com"]
fn transport_reaches_the_api_over_tls() {
    let response = HttpTransport::default()
        .send(
            &Request::get(format!("https://api.github.com/repos/{}/{}", REPO.0, REPO.1))
                .header("Accept", "application/vnd.github+json"),
        )
        .unwrap();
    assert_eq!(response.status, 200);
    assert!(response.header("x-ratelimit-limit").is_some());
    let repo: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(repo["name"], REPO.1);
}

#[test]
#[ignore = "network: needs `gh auth login`"]
fn lists_runs_jobs_and_a_log_with_the_gh_token() {
    let Some((token, _)) = ci::auth::resolve_token() else {
        return;
    };
    let github = GitHub::new(
        Arc::new(HttpTransport::default()),
        token,
        RepoId {
            owner: REPO.0.into(),
            name: REPO.1.into(),
        },
    );
    let runs = github
        .runs(&RunFilter {
            status: Some("completed".into()),
            per_page: 3,
            ..RunFilter::default()
        })
        .unwrap();
    let run = runs.value.first().expect("cargo has completed runs");
    assert!(
        !github
            .runs(&RunFilter {
                per_page: 3,
                status: Some("completed".into()),
                ..RunFilter::default()
            })
            .unwrap()
            .changed
    );
    let jobs = github.jobs(run.id).unwrap().value;
    let job = jobs.first().expect("a run has jobs");
    let log = github.job_log(job.id).unwrap();
    let parsed = ci::parse_log(&log, &job.steps);
    assert!(!parsed.lines.is_empty());
}
