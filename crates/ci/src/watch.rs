//! Which runs just finished and deserve a notification: runs on HEAD and runs started from
//! the IDE, only on the transition from queued/running to done (never on the first load).

use std::collections::HashMap;

use crate::models::{Run, State};

/// Run id → state at the previous poll.
pub type States = HashMap<u64, State>;

pub fn states(runs: &[Run]) -> States {
    runs.iter().map(|r| (r.id, r.state())).collect()
}

/// Runs in `runs` that `watched` cares about and that were active at the previous poll but are not now.
pub fn finished<'a>(previous: &States, runs: &'a [Run], watched: impl Fn(&Run) -> bool) -> Vec<&'a Run> {
    runs.iter()
        .filter(|run| watched(run))
        .filter(|run| !run.state().is_active())
        .filter(|run| previous.get(&run.id).is_some_and(|s| s.is_active()))
        .collect()
}

/// One line for a notification: "CI #12 failed on main · Fix the build".
pub fn summary(run: &Run) -> (String, String) {
    let verb = match run.state() {
        State::Success => "passed",
        State::Failure => "failed",
        State::Cancelled => "was cancelled",
        _ => "finished",
    };
    let title = format!("{} #{} {verb}", run.workflow_name(), run.run_number);
    let body = format!(
        "{} · {}",
        run.head_branch.as_deref().unwrap_or(run.short_sha()),
        run.commit_subject()
    );
    (title, body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(id: u64, sha: &str, status: &str, conclusion: Option<&str>) -> Run {
        serde_json::from_value(serde_json::json!({
            "id": id, "name": "CI", "run_number": 12, "head_branch": "main", "head_sha": sha,
            "event": "push", "status": status, "conclusion": conclusion, "workflow_id": 1,
            "html_url": "", "created_at": "", "updated_at": "",
            "head_commit": {"message": "Fix the build"}
        }))
        .unwrap()
    }

    #[test]
    fn reports_only_watched_runs_that_just_finished() {
        let before = vec![
            run(1, "head", "in_progress", None),
            run(2, "other", "in_progress", None),
            run(3, "head", "completed", Some("success")),
        ];
        let after = vec![
            run(1, "head", "completed", Some("failure")),
            run(2, "other", "completed", Some("failure")),
            run(3, "head", "completed", Some("success")),
            run(4, "head", "completed", Some("success")),
        ];
        let done = finished(&states(&before), &after, |r| r.head_sha == "head");
        assert_eq!(done.iter().map(|r| r.id).collect::<Vec<_>>(), [1]);
        assert!(finished(&States::new(), &after, |_| true).is_empty());
    }

    #[test]
    fn summarizes_a_run() {
        let (title, body) = summary(&run(1, "head", "completed", Some("failure")));
        assert_eq!(title, "CI #12 failed");
        assert_eq!(body, "main · Fix the build");
    }
}
