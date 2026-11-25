//! The GitHub client against a scripted transport.
use super::*;
use crate::transport::FakeTransport;

fn client(fake: &Arc<FakeTransport>) -> GitHub {
    GitHub::new(
        fake.clone(),
        Token::new("ghp_secret"),
        RepoId {
            owner: "o".into(),
            name: "r".into(),
        },
    )
}

const RUNS: &str = r#"{"total_count": 1, "workflow_runs": [{"id": 1, "name": "CI", "head_sha": "abc",
    "event": "push", "status": "in_progress", "conclusion": null, "workflow_id": 9,
    "html_url": "u", "created_at": "2026-11-25T10:00:00Z", "updated_at": "2026-11-25T10:00:00Z"}]}"#;

#[test]
fn sends_auth_and_version_headers_and_filters() {
    let fake = Arc::new(FakeTransport::default());
    fake.push(200, &[], RUNS);
    let github = client(&fake);
    let runs = github
        .runs(&RunFilter {
            branch: Some("feature/a b".into()),
            ..RunFilter::default()
        })
        .unwrap();
    assert_eq!(runs.value.len(), 1);
    let request = &fake.requests()[0];
    assert_eq!(
        request.url,
        "https://api.github.com/repos/o/r/actions/runs?per_page=20&branch=feature/a%20b"
    );
    assert!(
        request
            .headers
            .contains(&("Authorization".into(), "Bearer ghp_secret".into()))
    );
    assert!(
        request
            .headers
            .contains(&("X-GitHub-Api-Version".into(), "2022-11-28".into()))
    );
}

#[test]
fn revalidates_with_etag_and_reuses_the_body_on_304() {
    let fake = Arc::new(FakeTransport::default());
    fake.push(
        200,
        &[("ETag", "W/\"1\""), ("x-ratelimit-remaining", "4999")],
        RUNS,
    );
    fake.push(304, &[("x-ratelimit-remaining", "4999")], "");
    let github = client(&fake);
    let first = github.runs(&RunFilter::default()).unwrap();
    let second = github.runs(&RunFilter::default()).unwrap();
    assert!(first.changed);
    assert!(!second.changed);
    assert_eq!(first.value, second.value);
    let requests = fake.requests();
    assert!(!requests[0].has_header("If-None-Match"));
    assert!(
        requests[1]
            .headers
            .contains(&("If-None-Match".into(), "W/\"1\"".into()))
    );
    assert_eq!(github.rate_limit().map(|r| r.remaining), Some(4999));
}

#[test]
fn follows_the_log_redirect_without_the_token() {
    let fake = Arc::new(FakeTransport::default());
    fake.push(302, &[("Location", "https://logs.example/job?sig=1")], "");
    fake.push(200, &[], "2026-11-25T10:00:00.0000000Z hello\n");
    let log = client(&fake).job_log(5).unwrap();
    assert!(log.contains("hello"));
    let requests = fake.requests();
    assert!(requests[0].has_header("Authorization"));
    assert_eq!(requests[1].url, "https://logs.example/job?sig=1");
    assert!(!requests[1].has_header("Authorization"));
}

#[test]
fn maps_statuses_to_friendly_errors() {
    let fake = Arc::new(FakeTransport::default());
    fake.push(401, &[], r#"{"message": "Bad credentials"}"#);
    fake.push(
        403,
        &[("x-ratelimit-remaining", "0"), ("x-ratelimit-reset", "100")],
        "{}",
    );
    fake.push(
        403,
        &[],
        r#"{"message": "Resource not accessible by integration"}"#,
    );
    fake.push(404, &[], r#"{"message": "Not Found"}"#);
    fake.push(500, &[], "oops");
    let github = client(&fake);
    let filter = RunFilter::default();
    assert!(matches!(github.runs(&filter), Err(ApiError::Unauthorized)));
    assert!(matches!(
        github.runs(&filter),
        Err(ApiError::RateLimited { reset_at: Some(100) })
    ));
    let forbidden = github.runs(&filter).unwrap_err();
    assert!(forbidden.to_string().contains("Resource not accessible"));
    assert!(matches!(github.runs(&filter), Err(ApiError::NotFound)));
    let server = github.runs(&filter).unwrap_err();
    assert!(server.is_unexpected());
    assert!(!server.to_string().contains("ghp_secret"));
}

#[test]
fn posts_run_actions_and_dispatches() {
    let fake = Arc::new(FakeTransport::default());
    fake.push(201, &[], "");
    fake.push(201, &[], "");
    fake.push(202, &[], "");
    fake.push(204, &[], "");
    fake.push(
        422,
        &[],
        r#"{"message": "Workflow does not have 'workflow_dispatch' trigger"}"#,
    );
    let github = client(&fake);
    github.rerun(7).unwrap();
    github.rerun_failed(7).unwrap();
    github.cancel(7).unwrap();
    let mut inputs = serde_json::Map::new();
    inputs.insert("env".into(), "staging".into());
    assert_eq!(github.dispatch(3, "main", inputs.clone()).unwrap(), None);
    let err = github.dispatch(3, "main", inputs).unwrap_err();
    assert!(err.to_string().contains("workflow_dispatch"));
    let requests = fake.requests();
    let urls: Vec<&str> = requests.iter().map(|r| r.url.as_str()).collect();
    assert_eq!(
        urls[..4],
        [
            "https://api.github.com/repos/o/r/actions/runs/7/rerun",
            "https://api.github.com/repos/o/r/actions/runs/7/rerun-failed-jobs",
            "https://api.github.com/repos/o/r/actions/runs/7/cancel",
            "https://api.github.com/repos/o/r/actions/workflows/3/dispatches",
        ]
    );
    assert!(requests.iter().all(|r| r.method == "POST"));
    let body: serde_json::Value = serde_json::from_slice(requests[3].body.as_ref().unwrap()).unwrap();
    assert_eq!(
        body,
        serde_json::json!({"ref": "main", "inputs": {"env": "staging"}})
    );
}

#[test]
fn dispatch_returns_the_run_id_when_github_sends_it() {
    let fake = Arc::new(FakeTransport::default());
    fake.push(200, &[], r#"{"workflow_run_id": 99, "run_url": "u"}"#);
    assert_eq!(
        client(&fake).dispatch(3, "main", serde_json::Map::new()).unwrap(),
        Some(99)
    );
}

#[test]
fn finds_the_run_a_dispatch_created() {
    let fake = Arc::new(FakeTransport::default());
    let run = |id: u64, created: &str| {
        format!(
            r#"{{"id": {id}, "head_sha": "a", "event": "workflow_dispatch", "status": "queued",
                "conclusion": null, "workflow_id": 3, "html_url": "u", "created_at": "{created}",
                "updated_at": "{created}"}}"#
        )
    };
    let body = format!(
        r#"{{"workflow_runs": [{}, {}]}}"#,
        run(5, "2026-11-25T10:00:00Z"),
        run(4, "2026-11-25T09:00:00Z")
    );
    fake.push(200, &[], &body);
    fake.push(200, &[], &body);
    let github = client(&fake);
    let at = crate::time::parse_rfc3339("2026-11-25T10:00:10Z").unwrap();
    assert_eq!(
        github.find_dispatched_run(3, "feat/x", at).unwrap().map(|r| r.id),
        Some(5)
    );
    let later = crate::time::parse_rfc3339("2026-11-25T11:00:00Z").unwrap();
    assert_eq!(github.find_dispatched_run(3, "feat/x", later).unwrap(), None);
    assert!(
        fake.requests()[0]
            .url
            .ends_with("event=workflow_dispatch&per_page=5&branch=feat/x")
    );
}

#[test]
fn reads_a_raw_file_at_a_ref() {
    let fake = Arc::new(FakeTransport::default());
    fake.push(200, &[], "on: workflow_dispatch\n");
    let text = client(&fake)
        .file(".github/workflows/deploy.yml", "main")
        .unwrap();
    assert!(text.starts_with("on:"));
    let request = &fake.requests()[0];
    assert!(
        request
            .url
            .ends_with("/contents/.github/workflows/deploy.yml?ref=main")
    );
    assert!(
        request
            .headers
            .contains(&("Accept".into(), "application/vnd.github.raw+json".into()))
    );
    assert_eq!(request.headers.iter().filter(|(n, _)| n == "Accept").count(), 1);
}

#[test]
fn debug_output_never_shows_the_token() {
    let fake = Arc::new(FakeTransport::default());
    let github = client(&fake);
    assert!(!format!("{github:?}").contains("ghp_secret"));
    let request = github.authorized(Request::get("u"));
    assert!(!format!("{request:?}").contains("ghp_secret"));
}
