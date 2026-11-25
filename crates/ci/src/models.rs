//! GitHub Actions REST objects (only the fields the panel uses) and their derived state.

use serde::Deserialize;

use crate::time::{Millis, parse_rfc3339};

/// What a run, job or step is doing or how it ended, folded from `status` + `conclusion`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum State {
    Queued,
    Running,
    Success,
    Failure,
    Cancelled,
    Skipped,
    /// Neutral, stale or an unknown conclusion.
    Neutral,
}

impl State {
    pub fn from_api(status: &str, conclusion: Option<&str>) -> Self {
        match status {
            "queued" | "waiting" | "pending" | "requested" => Self::Queued,
            "in_progress" => Self::Running,
            _ => match conclusion {
                Some("success") => Self::Success,
                Some("failure" | "timed_out" | "startup_failure" | "action_required") => Self::Failure,
                Some("cancelled") => Self::Cancelled,
                Some("skipped") => Self::Skipped,
                // Completed without a conclusion yet: GitHub is still finishing up.
                None if status != "completed" => Self::Running,
                _ => Self::Neutral,
            },
        }
    }

    pub fn is_active(self) -> bool {
        matches!(self, Self::Queued | Self::Running)
    }

    /// The glyph the panel shows next to a run, job or step.
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Queued => "○",
            Self::Running => "●",
            Self::Success => "✓",
            Self::Failure => "✗",
            Self::Cancelled => "⊘",
            Self::Skipped => "–",
            Self::Neutral => "·",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Queued => "Queued",
            Self::Running => "Running",
            Self::Success => "Success",
            Self::Failure => "Failed",
            Self::Cancelled => "Cancelled",
            Self::Skipped => "Skipped",
            Self::Neutral => "Neutral",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Workflow {
    pub id: u64,
    pub name: String,
    pub path: String,
    pub state: String,
    #[serde(default)]
    pub html_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct WorkflowList {
    pub workflows: Vec<Workflow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Actor {
    pub login: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct HeadCommit {
    #[serde(default)]
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Run {
    pub id: u64,
    /// The workflow name.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub display_title: String,
    #[serde(default)]
    pub head_branch: Option<String>,
    pub head_sha: String,
    pub event: String,
    pub status: Option<String>,
    pub conclusion: Option<String>,
    pub workflow_id: u64,
    pub html_url: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub run_started_at: Option<String>,
    #[serde(default)]
    pub run_number: u64,
    #[serde(default)]
    pub run_attempt: u64,
    #[serde(default)]
    pub actor: Option<Actor>,
    #[serde(default)]
    pub head_commit: Option<HeadCommit>,
}

impl Run {
    pub fn state(&self) -> State {
        State::from_api(
            self.status.as_deref().unwrap_or_default(),
            self.conclusion.as_deref(),
        )
    }

    pub fn workflow_name(&self) -> &str {
        self.name.as_deref().unwrap_or("Workflow")
    }

    pub fn short_sha(&self) -> &str {
        self.head_sha.get(..7).unwrap_or(&self.head_sha)
    }

    /// First line of the head commit message (falls back to the run title).
    pub fn commit_subject(&self) -> &str {
        self.head_commit
            .as_ref()
            .and_then(|c| c.message.lines().next())
            .unwrap_or(&self.display_title)
    }

    pub fn created_millis(&self) -> Option<Millis> {
        parse_rfc3339(&self.created_at)
    }

    /// Wall time of the latest attempt: until now while it is active.
    pub fn duration_millis(&self, now: Millis) -> Option<Millis> {
        let start = parse_rfc3339(self.run_started_at.as_deref().unwrap_or(&self.created_at))?;
        let end = if self.state().is_active() {
            now
        } else {
            parse_rfc3339(&self.updated_at)?
        };
        Some((end - start).max(0))
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RunList {
    pub workflow_runs: Vec<Run>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Step {
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub number: u32,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub completed_at: Option<String>,
}

impl Step {
    pub fn state(&self) -> State {
        State::from_api(&self.status, self.conclusion.as_deref())
    }

    pub fn duration_millis(&self, now: Millis) -> Option<Millis> {
        span(
            self.started_at.as_deref(),
            self.completed_at.as_deref(),
            self.state(),
            now,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Job {
    pub id: u64,
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub completed_at: Option<String>,
    #[serde(default)]
    pub html_url: Option<String>,
    #[serde(default)]
    pub steps: Vec<Step>,
}

impl Job {
    pub fn state(&self) -> State {
        State::from_api(&self.status, self.conclusion.as_deref())
    }

    pub fn duration_millis(&self, now: Millis) -> Option<Millis> {
        span(
            self.started_at.as_deref(),
            self.completed_at.as_deref(),
            self.state(),
            now,
        )
    }

    /// The first failed step, the one the panel opens on.
    pub fn failed_step(&self) -> Option<&Step> {
        self.steps.iter().find(|s| s.state() == State::Failure)
    }
}

fn span(start: Option<&str>, end: Option<&str>, state: State, now: Millis) -> Option<Millis> {
    let start = parse_rfc3339(start?)?;
    let end = match end.and_then(parse_rfc3339) {
        Some(end) => end,
        None if state == State::Running => now,
        None => return None,
    };
    Some((end - start).max(0))
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct JobList {
    pub jobs: Vec<Job>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Annotation {
    pub path: String,
    pub start_line: u32,
    #[serde(default)]
    pub end_line: u32,
    /// `notice`, `warning` or `failure`.
    pub annotation_level: String,
    pub message: String,
    #[serde(default)]
    pub title: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_status_and_conclusion() {
        assert_eq!(State::from_api("queued", None), State::Queued);
        assert_eq!(State::from_api("waiting", None), State::Queued);
        assert_eq!(State::from_api("in_progress", None), State::Running);
        assert_eq!(State::from_api("completed", Some("success")), State::Success);
        assert_eq!(State::from_api("completed", Some("timed_out")), State::Failure);
        assert_eq!(State::from_api("completed", Some("cancelled")), State::Cancelled);
        assert_eq!(State::from_api("completed", Some("skipped")), State::Skipped);
        assert_eq!(State::from_api("completed", Some("stale")), State::Neutral);
    }

    #[test]
    fn parses_a_run_from_the_api() {
        let run: Run = serde_json::from_str(
            r#"{"id": 7, "name": "CI", "display_title": "Fix it", "head_branch": "main",
                "head_sha": "abcdef1234567", "event": "push", "status": "completed",
                "conclusion": "failure", "workflow_id": 3, "html_url": "https://github.com/o/r/actions/runs/7",
                "created_at": "2026-11-25T10:00:00Z", "updated_at": "2026-11-25T10:03:05Z",
                "run_started_at": "2026-11-25T10:00:00Z", "run_number": 12, "run_attempt": 1,
                "actor": {"login": "me"}, "head_commit": {"message": "Fix the build\n\nDetails"}}"#,
        )
        .unwrap();
        assert_eq!(run.state(), State::Failure);
        assert_eq!(run.short_sha(), "abcdef1");
        assert_eq!(run.commit_subject(), "Fix the build");
        assert_eq!(run.duration_millis(0), Some(185_000));
    }

    #[test]
    fn running_steps_tick_until_now() {
        let step = Step {
            name: "Test".into(),
            status: "in_progress".into(),
            conclusion: None,
            number: 3,
            started_at: Some("2026-11-25T10:00:00Z".into()),
            completed_at: None,
        };
        let start = parse_rfc3339("2026-11-25T10:00:00Z").unwrap();
        assert_eq!(step.duration_millis(start + 5_000), Some(5_000));
    }
}
