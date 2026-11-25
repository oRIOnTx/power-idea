//! The run detail as one flat, virtualized list: job headers, their steps, and the log lines
//! of expanded steps (like GitHub's own job view), plus what the HEAD badge shows.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;

use ci::{Annotation, Job, ParsedLog, Run, State};

/// A job's log: fetched once the job has finished.
#[derive(Debug, Clone)]
pub enum LogState {
    Loading,
    Ready(Arc<ParsedLog>),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Note {
    Loading,
    /// Job logs exist only after the job ends; until then the steps show progress.
    StillRunning,
    NoOutput,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetailRow {
    Job {
        job: usize,
    },
    /// A failure or warning annotation (file:line → message) under its job.
    Annotation {
        job: usize,
        ix: usize,
    },
    Step {
        job: usize,
        step: usize,
    },
    Log {
        job: usize,
        line: usize,
    },
    Note {
        job: usize,
        note: Note,
    },
}

impl DetailRow {
    pub fn job(&self) -> usize {
        match self {
            Self::Job { job }
            | Self::Annotation { job, .. }
            | Self::Step { job, .. }
            | Self::Log { job, .. }
            | Self::Note { job, .. } => *job,
        }
    }
}

/// Folding state of the detail tree.
#[derive(Debug, Clone, Default)]
pub struct Expansion {
    pub jobs: HashSet<u64>,
    /// (job id, step number)
    pub steps: HashSet<(u64, u32)>,
}

/// What the detail list is built from.
#[derive(Debug, Clone, Copy)]
pub struct DetailSource<'a> {
    pub jobs: &'a [Job],
    pub expansion: &'a Expansion,
    pub logs: &'a HashMap<u64, LogState>,
    pub annotations: &'a HashMap<u64, Vec<Annotation>>,
}

/// Annotations worth showing: failures and warnings, not notices.
pub fn shown_annotations(annotations: &[Annotation]) -> impl Iterator<Item = (usize, &Annotation)> {
    annotations
        .iter()
        .enumerate()
        .filter(|(_, a)| a.annotation_level != "notice")
}

pub fn detail_rows(source: DetailSource<'_>) -> Vec<DetailRow> {
    let DetailSource {
        jobs,
        expansion,
        logs,
        annotations,
    } = source;
    let mut rows = Vec::new();
    for (j, job) in jobs.iter().enumerate() {
        rows.push(DetailRow::Job { job: j });
        if !expansion.jobs.contains(&job.id) {
            continue;
        }
        if let Some(list) = annotations.get(&job.id) {
            rows.extend(shown_annotations(list).map(|(ix, _)| DetailRow::Annotation { job: j, ix }));
        }
        for (s, step) in job.steps.iter().enumerate() {
            rows.push(DetailRow::Step { job: j, step: s });
            if !expansion.steps.contains(&(job.id, step.number)) {
                continue;
            }
            if job.state().is_active() {
                rows.push(DetailRow::Note {
                    job: j,
                    note: Note::StillRunning,
                });
                continue;
            }
            match logs.get(&job.id) {
                None | Some(LogState::Loading) => rows.push(DetailRow::Note {
                    job: j,
                    note: Note::Loading,
                }),
                Some(LogState::Failed(message)) => rows.push(DetailRow::Note {
                    job: j,
                    note: Note::Failed(message.clone()),
                }),
                Some(LogState::Ready(log)) => match step_lines(log, step.number) {
                    Some(range) if !range.is_empty() => {
                        rows.extend(range.map(|line| DetailRow::Log { job: j, line }));
                    }
                    _ => rows.push(DetailRow::Note {
                        job: j,
                        note: Note::NoOutput,
                    }),
                },
            }
        }
    }
    rows
}

/// Lines of `step`; the whole log when it could not be split into steps.
pub fn step_lines(log: &ParsedLog, step: u32) -> Option<Range<usize>> {
    if log.sections.is_empty() {
        return Some(0..log.lines.len());
    }
    log.section(step).map(|s| s.lines.clone())
}

/// What to unfold when a run is first shown: failed and running jobs, and each failed step.
pub fn default_expansion(jobs: &[Job], expansion: &mut Expansion) {
    for job in jobs {
        let state = job.state();
        if matches!(state, State::Failure | State::Running) {
            expansion.jobs.insert(job.id);
        }
        if let Some(step) = job.failed_step() {
            expansion.steps.insert((job.id, step.number));
        }
    }
}

/// Rows whose log line contains `query` (case-insensitive).
pub fn search_matches(
    rows: &[DetailRow],
    jobs: &[Job],
    logs: &HashMap<u64, LogState>,
    query: &str,
) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    rows.iter()
        .enumerate()
        .filter_map(|(ix, row)| {
            let DetailRow::Log { job, line } = row else {
                return None;
            };
            let Some(LogState::Ready(log)) = jobs.get(*job).and_then(|j| logs.get(&j.id)) else {
                return None;
            };
            log.lines
                .get(*line)
                .is_some_and(|l| l.text.to_lowercase().contains(&query))
                .then_some(ix)
        })
        .collect()
}

/// The CI state of HEAD: the latest run of each workflow on that commit, folded worst-first.
pub fn head_state(runs: &[Run], head_sha: &str) -> Option<State> {
    let mut seen = HashSet::new();
    let latest: Vec<State> = runs
        .iter()
        .filter(|r| r.head_sha == head_sha)
        .filter(|r| seen.insert(r.workflow_id))
        .map(Run::state)
        .collect();
    let any = |s: State| latest.contains(&s);
    if latest.is_empty() {
        None
    } else if any(State::Failure) {
        Some(State::Failure)
    } else if any(State::Running) {
        Some(State::Running)
    } else if any(State::Queued) {
        Some(State::Queued)
    } else if any(State::Cancelled) {
        Some(State::Cancelled)
    } else {
        Some(State::Success)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use ci::{Annotation, Job, Run, State, Step, parse_log};

    use super::{
        DetailRow, DetailSource, Expansion, LogState, Note, default_expansion, detail_rows, head_state,
        search_matches,
    };

    fn rows_of(
        jobs: &[Job],
        expansion: &Expansion,
        logs: &HashMap<u64, LogState>,
        annotations: &HashMap<u64, Vec<Annotation>>,
    ) -> Vec<DetailRow> {
        detail_rows(DetailSource {
            jobs,
            expansion,
            logs,
            annotations,
        })
    }

    fn step(number: u32, conclusion: &str, start: &str, end: &str) -> Step {
        Step {
            name: format!("step {number}"),
            status: "completed".into(),
            conclusion: Some(conclusion.into()),
            number,
            started_at: Some(format!("2026-11-25T10:00:{start}Z")),
            completed_at: Some(format!("2026-11-25T10:00:{end}Z")),
        }
    }

    fn job(id: u64, status: &str, conclusion: Option<&str>, steps: Vec<Step>) -> Job {
        Job {
            id,
            name: format!("job {id}"),
            status: status.into(),
            conclusion: conclusion.map(str::to_string),
            started_at: None,
            completed_at: None,
            html_url: None,
            steps,
        }
    }

    fn jobs() -> Vec<Job> {
        vec![
            job(
                1,
                "completed",
                Some("success"),
                vec![step(1, "success", "00", "01")],
            ),
            job(
                2,
                "completed",
                Some("failure"),
                vec![step(1, "success", "00", "01"), step(2, "failure", "02", "05")],
            ),
            job(3, "in_progress", None, vec![step(1, "success", "00", "01")]),
        ]
    }

    const LOG: &str = "2026-11-25T10:00:00.1000000Z setup\n\
                       2026-11-25T10:00:02.1000000Z ##[group]Run tests\n\
                       2026-11-25T10:00:03.1000000Z boom Error: bad\n\
                       2026-11-25T10:00:04.1000000Z ##[error]Process completed with exit code 1.\n";

    #[test]
    fn unfolds_failed_and_running_jobs_and_the_failed_step() {
        let jobs = jobs();
        let mut expansion = Expansion::default();
        default_expansion(&jobs, &mut expansion);
        assert!(!expansion.jobs.contains(&1));
        assert!(expansion.jobs.contains(&2) && expansion.jobs.contains(&3));
        assert_eq!(expansion.steps.iter().copied().collect::<Vec<_>>(), [(2, 2)]);
    }

    #[test]
    fn flattens_jobs_steps_and_the_failed_step_log() {
        let jobs = jobs();
        let mut expansion = Expansion::default();
        default_expansion(&jobs, &mut expansion);
        let mut logs = HashMap::new();
        let mut annotations = HashMap::new();
        assert!(
            rows_of(&jobs, &expansion, &logs, &annotations).contains(&DetailRow::Note {
                job: 1,
                note: Note::Loading
            })
        );
        logs.insert(2, LogState::Ready(Arc::new(parse_log(LOG, &jobs[1].steps))));
        let annotation = |level: &str| Annotation {
            path: "spec/a_spec.rb".into(),
            start_line: 4,
            end_line: 4,
            annotation_level: level.into(),
            message: "boom".into(),
            title: None,
        };
        annotations.insert(2, vec![annotation("notice"), annotation("failure")]);
        let rows = rows_of(&jobs, &expansion, &logs, &annotations);
        assert_eq!(
            rows,
            [
                DetailRow::Job { job: 0 },
                DetailRow::Job { job: 1 },
                DetailRow::Annotation { job: 1, ix: 1 },
                DetailRow::Step { job: 1, step: 0 },
                DetailRow::Step { job: 1, step: 1 },
                DetailRow::Log { job: 1, line: 1 },
                DetailRow::Log { job: 1, line: 2 },
                DetailRow::Log { job: 1, line: 3 },
                DetailRow::Job { job: 2 },
                DetailRow::Step { job: 2, step: 0 },
            ]
        );
        assert_eq!(search_matches(&rows, &jobs, &logs, "ERROR:"), [6]);
        assert!(search_matches(&rows, &jobs, &logs, "  ").is_empty());
    }

    #[test]
    fn running_jobs_say_logs_come_later() {
        let jobs = jobs();
        let mut expansion = Expansion::default();
        expansion.jobs.insert(3);
        expansion.steps.insert((3, 1));
        let rows = rows_of(&jobs, &expansion, &HashMap::new(), &HashMap::new());
        assert_eq!(
            rows.last(),
            Some(&DetailRow::Note {
                job: 2,
                note: Note::StillRunning
            })
        );
    }

    fn run(id: u64, workflow: u64, sha: &str, status: &str, conclusion: Option<&str>) -> Run {
        serde_json::from_value(serde_json::json!({
            "id": id, "head_sha": sha, "event": "push", "status": status, "conclusion": conclusion,
            "workflow_id": workflow, "html_url": "", "created_at": "", "updated_at": ""
        }))
        .unwrap()
    }

    #[test]
    fn head_state_uses_the_latest_run_per_workflow() {
        let runs = vec![
            run(4, 1, "abc", "completed", Some("success")),
            run(3, 1, "abc", "completed", Some("failure")),
            run(2, 2, "abc", "in_progress", None),
            run(1, 3, "old", "completed", Some("failure")),
        ];
        assert_eq!(head_state(&runs, "abc"), Some(State::Running));
        assert_eq!(head_state(&runs[..2], "abc"), Some(State::Success));
        assert_eq!(head_state(&runs, "old"), Some(State::Failure));
        assert_eq!(head_state(&runs, "none"), None);
    }
}
