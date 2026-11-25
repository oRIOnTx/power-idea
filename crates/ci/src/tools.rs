//! Answers of the IDE's read-only MCP tools for GitHub Actions: recent runs, one run with its
//! jobs, steps and annotations, and the trimmed logs of failed steps. Everything is redacted.

use serde_json::{Value, json};

use crate::context::{self, Failure};
use crate::github::{GitHub, RunFilter};
use crate::models::{Job, Run, State};
use crate::redact::redact;

pub const LIST_RUNS: &str = "ci_list_runs";
pub const GET_RUN: &str = "ci_get_run";
pub const FAILED_LOGS: &str = "ci_get_failed_logs";

/// A failed job's block is never trimmed below this, however many jobs failed.
const MIN_PER_JOB: usize = 3_000;

fn run_json(run: &Run) -> Value {
    json!({
        "id": run.id,
        "workflow": run.workflow_name(),
        "title": run.commit_subject(),
        "branch": run.head_branch,
        "sha": run.head_sha,
        "event": run.event,
        "status": status(run.state()),
        "attempt": run.run_attempt,
        "created_at": run.created_at,
        "url": run.html_url,
        "actor": run.actor.as_ref().map(|a| a.login.clone()),
    })
}

fn status(state: State) -> &'static str {
    match state {
        State::Queued => "queued",
        State::Running => "in_progress",
        State::Success => "success",
        State::Failure => "failure",
        State::Cancelled => "cancelled",
        State::Skipped => "skipped",
        State::Neutral => "neutral",
    }
}

fn pretty(value: &Value) -> Result<String, String> {
    serde_json::to_string_pretty(value)
        .map(|text| redact(&text))
        .map_err(|e| e.to_string())
}

/// `ci_list_runs(branch?, limit?)`: newest first; all branches when `branch` is empty.
pub fn list_runs(client: &GitHub, branch: Option<&str>, limit: usize) -> Result<String, String> {
    let filter = RunFilter {
        branch: branch.filter(|b| !b.is_empty()).map(str::to_string),
        per_page: u32::try_from(limit.clamp(1, 50)).unwrap_or(10),
        ..RunFilter::default()
    };
    let runs = client.runs(&filter).map_err(|e| e.to_string())?.value;
    pretty(&json!(runs.iter().map(run_json).collect::<Vec<_>>()))
}

/// `ci_get_run(run_id)`: the run, its jobs and steps, and annotations of failed jobs.
pub fn get_run(client: &GitHub, run_id: u64) -> Result<String, String> {
    let run = client.run(run_id).map_err(|e| e.to_string())?.value;
    let jobs = client.jobs(run_id).map_err(|e| e.to_string())?.value;
    let jobs: Vec<Value> = jobs.iter().map(|job| job_json(client, job)).collect();
    pretty(&json!({ "run": run_json(&run), "jobs": jobs }))
}

fn job_json(client: &GitHub, job: &Job) -> Value {
    let annotations: Vec<Value> = if job.state() == State::Failure {
        client
            .annotations(job.id)
            .unwrap_or_default()
            .iter()
            .filter(|a| a.annotation_level != "notice")
            .map(|a| json!({ "path": a.path, "line": a.start_line, "level": a.annotation_level, "message": a.message }))
            .collect()
    } else {
        Vec::new()
    };
    json!({
        "id": job.id,
        "name": job.name,
        "status": status(job.state()),
        "url": job.html_url,
        "steps": job.steps.iter().map(|s| json!({
            "number": s.number,
            "name": s.name,
            "status": status(s.state()),
        })).collect::<Vec<_>>(),
        "annotations": annotations,
    })
}

/// `ci_get_failed_logs(run_id)`: one Markdown block per failed job (as "Send to Claude"
/// builds it), sharing `limit` characters.
pub fn failed_logs(client: &GitHub, run_id: u64, limit: usize) -> Result<String, String> {
    let run = client.run(run_id).map_err(|e| e.to_string())?.value;
    let jobs = client.jobs(run_id).map_err(|e| e.to_string())?.value;
    let failed: Vec<&Job> = context::failed_jobs(&jobs).collect();
    if failed.is_empty() {
        return Ok(format!(
            "Run {run_id} ({} #{}) has no failed jobs; status: {}.",
            run.workflow_name(),
            run.run_number,
            status(run.state())
        ));
    }
    let per_job = (limit / failed.len()).max(MIN_PER_JOB);
    let blocks: Vec<String> = failed
        .iter()
        .map(|job| {
            let log = client
                .job_log(job.id)
                .ok()
                .map(|raw| crate::parse_log(&raw, &job.steps));
            let annotations = client.annotations(job.id).unwrap_or_default();
            context::build(&Failure {
                repo: client.repo(),
                run: &run,
                job,
                step: None,
                annotations: &annotations,
                log: log.as_ref(),
                limit: per_job,
            })
        })
        .collect();
    Ok(blocks.join("\n---\n\n"))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::auth::Token;
    use crate::remote::RepoId;
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

    const RUN: &str = r#"{"id": 7, "name": "CI", "run_number": 3, "head_branch": "main", "head_sha": "abc1234",
        "event": "push", "status": "completed", "conclusion": "failure", "workflow_id": 1,
        "html_url": "https://github.com/o/r/actions/runs/7", "created_at": "2026-11-25T10:00:00Z",
        "updated_at": "2026-11-25T10:01:00Z", "head_commit": {"message": "Break it"}}"#;

    const JOBS: &str = r#"{"jobs": [
        {"id": 1, "name": "lint", "status": "completed", "conclusion": "success", "steps": []},
        {"id": 2, "name": "test", "status": "completed", "conclusion": "failure", "steps": [
            {"name": "Run tests", "status": "completed", "conclusion": "failure", "number": 1,
             "started_at": "2026-11-25T10:00:00Z", "completed_at": "2026-11-25T10:00:30Z"}]}]}"#;

    #[test]
    fn lists_runs_compactly() {
        let fake = Arc::new(FakeTransport::default());
        fake.push(200, &[], &format!(r#"{{"workflow_runs": [{RUN}]}}"#));
        let text = list_runs(&client(&fake), Some("main"), 500).unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value[0]["status"], "failure");
        assert_eq!(value[0]["title"], "Break it");
        assert!(fake.requests()[0].url.ends_with("per_page=50&branch=main"));
    }

    #[test]
    fn gets_a_run_with_jobs_and_failed_annotations() {
        let fake = Arc::new(FakeTransport::default());
        fake.push(200, &[], RUN);
        fake.push(200, &[], JOBS);
        fake.push(
            200,
            &[],
            r#"[{"path": "spec/a_spec.rb", "start_line": 4, "annotation_level": "failure", "message": "boom"}]"#,
        );
        let value: Value = serde_json::from_str(&get_run(&client(&fake), 7).unwrap()).unwrap();
        assert_eq!(value["run"]["id"], 7);
        assert_eq!(value["jobs"][0]["annotations"], json!([]));
        assert_eq!(value["jobs"][1]["annotations"][0]["line"], 4);
        assert_eq!(value["jobs"][1]["steps"][0]["status"], "failure");
        // Annotations are fetched for the failed job only.
        assert_eq!(fake.requests().len(), 3);
    }

    #[test]
    fn failed_logs_are_trimmed_and_redacted_blocks() {
        let fake = Arc::new(FakeTransport::default());
        fake.push(200, &[], RUN);
        fake.push(200, &[], JOBS);
        fake.push(302, &[("Location", "https://logs.example/2")], "");
        fake.push(
            200,
            &[],
            "2026-11-25T10:00:01.0000000Z Error: expected true\n2026-11-25T10:00:02.0000000Z export TOKEN=ghp_abcdefghijklmnopqrstuvwxyz\n",
        );
        fake.push(200, &[], "[]");
        let text = failed_logs(&client(&fake), 7, 12_000).unwrap();
        assert!(text.starts_with("CI failure: CI / test / Run tests"));
        assert!(text.contains("[error] Error: expected true"));
        assert!(!text.contains("ghp_abc"));
    }

    #[test]
    fn says_when_nothing_failed() {
        let fake = Arc::new(FakeTransport::default());
        fake.push(200, &[], &RUN.replace("\"failure\"", "\"success\""));
        fake.push(200, &[], r#"{"jobs": []}"#);
        let text = failed_logs(&client(&fake), 7, 12_000).unwrap();
        assert_eq!(text, "Run 7 (CI #3) has no failed jobs; status: success.");
    }
}
