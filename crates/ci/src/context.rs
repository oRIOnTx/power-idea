//! A failed job as compact Markdown for Claude: where it failed, annotations, and the relevant
//! part of the failed step's log (error lines with context first), trimmed to a size limit
//! and with secrets redacted.

use std::fmt::Write as _;
use std::ops::Range;

use crate::log_parser::{LineKind, ParsedLog};
use crate::models::{Annotation, Job, Run, State, Step};
use crate::redact::redact;
use crate::remote::RepoId;

/// Default size of the block, in characters.
pub const DEFAULT_LIMIT: usize = 12_000;
/// Lines of context around each error line.
const CONTEXT: usize = 20;
/// The log excerpt never exceeds this many lines.
const MAX_LINES: usize = 150;

#[derive(Debug, Clone, Copy)]
pub struct Failure<'a> {
    pub repo: &'a RepoId,
    pub run: &'a Run,
    pub job: &'a Job,
    /// The step to explain; the job's first failed step when `None`.
    pub step: Option<&'a Step>,
    pub annotations: &'a [Annotation],
    pub log: Option<&'a ParsedLog>,
    pub limit: usize,
}

const TASK: &str = "Task: find the root cause in this repo and propose/apply a fix. If it's an \
                    environment/secrets issue, say so instead of changing code.";

pub fn build(failure: &Failure<'_>) -> String {
    let Failure {
        repo,
        run,
        job,
        annotations,
        log,
        limit,
        ..
    } = *failure;
    let step = failure.step.or_else(|| job.failed_step());
    let mut head = String::new();
    let _ = writeln!(
        head,
        "CI failure: {} / {}{}",
        run.workflow_name(),
        job.name,
        step.map_or_default(|s| format!(" / {}", s.name))
    );
    let _ = writeln!(
        head,
        "Repo: {repo}  Branch: {}  Commit: {} \"{}\"",
        run.head_branch.as_deref().unwrap_or("-"),
        run.short_sha(),
        run.commit_subject()
    );
    let _ = writeln!(head, "Run: {}", job.html_url.as_deref().unwrap_or(&run.html_url));
    let shown: Vec<&Annotation> = annotations
        .iter()
        .filter(|a| a.annotation_level != "notice")
        .take(20)
        .collect();
    if !shown.is_empty() {
        head.push_str("Annotations:\n");
        for a in shown {
            let message = a.message.lines().next().unwrap_or_default();
            let _ = writeln!(head, "- {}:{} — {message}", a.path, a.start_line);
        }
    }
    let tail = format!("\n{TASK}\n");
    let budget = limit.saturating_sub(head.len() + tail.len() + 200);
    let excerpt = log.map_or_default(|log| excerpt(log, step.map(|s| s.number), budget));
    let mut out = head;
    if excerpt.is_empty() {
        out.push_str("Log: not available (the job log could not be fetched or is empty).\n");
    } else {
        let fence = if excerpt.contains("```") { "~~~~" } else { "```" };
        let _ = write!(
            out,
            "Log (failed step, error lines ±{CONTEXT} lines of context):\n{fence}\n{excerpt}\n{fence}\n"
        );
    }
    out.push_str(&tail);
    redact(&out)
}

/// The step's (else the whole log's) error windows, merged, error sections first; the last
/// lines when there are no error lines. Omitted stretches are marked.
pub fn excerpt(log: &ParsedLog, step: Option<u32>, budget: usize) -> String {
    let range = step
        .and_then(|s| log.section(s).map(|sec| sec.lines.clone()))
        .unwrap_or(0..log.lines.len());
    let windows = windows(log, range.clone());
    let mut out = String::new();
    let mut used_lines = 0;
    let mut last_end = range.start;
    for window in windows {
        if window.start > last_end {
            let _ = writeln!(out, "… ({} lines omitted)", window.start - last_end);
        }
        for line in log.lines.get(window.clone()).unwrap_or_default() {
            let text = match line.kind {
                LineKind::Error => format!("[error] {}", line.text),
                LineKind::Warning => format!("[warning] {}", line.text),
                LineKind::Group => format!("▸ {}", line.text),
                _ => line.text.clone(),
            };
            if out.len() + text.len() + 1 > budget || used_lines >= MAX_LINES {
                out.push_str("… (trimmed)");
                return out;
            }
            out.push_str(&text);
            out.push('\n');
            used_lines += 1;
        }
        last_end = window.end;
    }
    if range.end > last_end && !out.is_empty() {
        let _ = writeln!(out, "… ({} lines omitted)", range.end - last_end);
    }
    out.trim_end().to_string()
}

/// Line ranges to show inside `range`: ±CONTEXT around error lines (merged, in order), or the
/// last MAX_LINES lines when nothing is marked as an error.
fn windows(log: &ParsedLog, range: Range<usize>) -> Vec<Range<usize>> {
    let errors: Vec<usize> = range
        .clone()
        .filter(|&ix| log.lines.get(ix).is_some_and(|l| l.kind == LineKind::Error))
        .collect();
    if errors.is_empty() {
        let tail = range.end.saturating_sub(MAX_LINES).max(range.start)..range.end;
        return vec![tail];
    }
    let mut windows: Vec<Range<usize>> = Vec::new();
    for ix in errors {
        let window = ix.saturating_sub(CONTEXT).max(range.start)..(ix + CONTEXT + 1).min(range.end);
        match windows.last_mut() {
            Some(last) if window.start <= last.end => last.end = last.end.max(window.end),
            _ => windows.push(window),
        }
    }
    windows
}

/// The failed jobs of a run, for "all failures" contexts (MCP).
pub fn failed_jobs(jobs: &[Job]) -> impl Iterator<Item = &Job> {
    jobs.iter().filter(|j| j.state() == State::Failure)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log_parser::parse_log;

    fn run() -> Run {
        serde_json::from_value(serde_json::json!({
            "id": 1, "name": "CI", "head_branch": "main", "head_sha": "abcdef123456", "event": "push",
            "status": "completed", "conclusion": "failure", "workflow_id": 1,
            "html_url": "https://github.com/o/r/actions/runs/1", "created_at": "", "updated_at": "",
            "head_commit": {"message": "Fix the build"}
        }))
        .unwrap()
    }

    fn job() -> Job {
        serde_json::from_value(serde_json::json!({
            "id": 5, "name": "test", "status": "completed", "conclusion": "failure",
            "html_url": "https://github.com/o/r/actions/runs/1/job/5",
            "steps": [
                {"name": "Checkout", "status": "completed", "conclusion": "success", "number": 1,
                 "started_at": "2026-11-25T10:00:00Z", "completed_at": "2026-11-25T10:00:01Z"},
                {"name": "Run tests", "status": "completed", "conclusion": "failure", "number": 2,
                 "started_at": "2026-11-25T10:00:02Z", "completed_at": "2026-11-25T10:00:09Z"}
            ]
        }))
        .unwrap()
    }

    fn log_with(lines: &[&str]) -> String {
        let mut out = String::from("2026-11-25T10:00:00.5000000Z checkout done\n");
        out.push_str("2026-11-25T10:00:02.1000000Z ##[group]Run tests\n");
        for line in lines {
            out.push_str("2026-11-25T10:00:03.0000000Z ");
            out.push_str(line);
            out.push('\n');
        }
        out
    }

    fn repo() -> RepoId {
        RepoId {
            owner: "o".into(),
            name: "r".into(),
        }
    }

    #[test]
    fn builds_the_block_with_header_annotations_log_and_task() {
        let (run, job, repo) = (run(), job(), repo());
        let raw = log_with(&[
            "ok 1",
            "Error: boom in user_spec.rb:42",
            "token=ghp_abcdefghijklmnopqrstuvwxyz",
        ]);
        let log = parse_log(&raw, &job.steps);
        let annotations = vec![Annotation {
            path: "spec/user_spec.rb".into(),
            start_line: 42,
            end_line: 42,
            annotation_level: "failure".into(),
            message: "expected valid\nmore".into(),
            title: None,
        }];
        let text = build(&Failure {
            repo: &repo,
            run: &run,
            job: &job,
            step: None,
            annotations: &annotations,
            log: Some(&log),
            limit: DEFAULT_LIMIT,
        });
        assert!(text.starts_with("CI failure: CI / test / Run tests\n"));
        assert!(text.contains("Repo: o/r  Branch: main  Commit: abcdef1 \"Fix the build\""));
        assert!(text.contains("Run: https://github.com/o/r/actions/runs/1/job/5"));
        assert!(text.contains("- spec/user_spec.rb:42 — expected valid\n"));
        assert!(text.contains("[error] Error: boom in user_spec.rb:42"));
        assert!(!text.contains("checkout done"), "other steps stay out");
        assert!(!text.contains("ghp_abc"));
        assert!(text.trim_end().ends_with("instead of changing code."));
    }

    #[test]
    fn keeps_error_windows_and_marks_what_is_left_out() {
        let mut lines: Vec<String> = (0..200).map(|i| format!("line {i}")).collect();
        lines[100] = "Error: first".into();
        lines[110] = "Error: second".into();
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let job = job();
        let log = parse_log(&log_with(&refs), &job.steps);
        let text = excerpt(&log, Some(2), DEFAULT_LIMIT);
        assert!(
            text.starts_with("… (81 lines omitted)\nline 80\n"),
            "{}",
            &text[..60]
        );
        assert!(text.contains("[error] Error: first") && text.contains("[error] Error: second"));
        assert!(text.contains("line 130\n… (69 lines omitted)"));
        assert!(!text.contains("line 131"));
    }

    #[test]
    fn falls_back_to_the_last_lines_and_respects_the_budget() {
        let lines: Vec<String> = (0..400).map(|i| format!("line {i}")).collect();
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let job = job();
        let log = parse_log(&log_with(&refs), &job.steps);
        let text = excerpt(&log, Some(2), DEFAULT_LIMIT);
        assert!(text.contains("line 399"));
        assert!(!text.contains("line 249\n"));
        let small = excerpt(&log, Some(2), 300);
        assert!(small.len() <= 300 + "… (trimmed)".len());
        assert!(small.ends_with("… (trimmed)"));
        let (run, repo) = (run(), repo());
        let block = build(&Failure {
            repo: &repo,
            run: &run,
            job: &job,
            step: None,
            annotations: &[],
            log: Some(&log),
            limit: 2_000,
        });
        assert!(block.len() <= 2_000, "{}", block.len());
    }
}
