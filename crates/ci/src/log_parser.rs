//! A GitHub Actions job log → display lines: timestamps and ANSI codes stripped, `##[…]`
//! workflow commands classified, error lines detected, and every line assigned to a step.
//!
//! The job log has no step markers, so lines are matched to steps by time: the jobs API gives
//! each step's start/end (whole seconds), each log line has a sub-second timestamp, and a new
//! step almost always opens with a top-level `##[group]`.

use std::ops::Range;

use crate::models::Step;
use crate::time::{Millis, parse_rfc3339};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Normal,
    Error,
    Warning,
    Notice,
    Debug,
    /// A command the runner executed (`##[command]`, `[command]`).
    Command,
    /// `##[group]Title`: a foldable header in GitHub's UI.
    Group,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    pub text: String,
    pub kind: LineKind,
    /// Nesting inside `##[group]` blocks (the header itself is at the outer depth).
    pub depth: u8,
    pub timestamp: Option<Millis>,
    /// Step `number` from the jobs API.
    pub step: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepSection {
    pub number: u32,
    pub lines: Range<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedLog {
    pub lines: Vec<LogLine>,
    /// In step order; steps with no lines are absent.
    pub sections: Vec<StepSection>,
}

impl ParsedLog {
    pub fn section(&self, step: u32) -> Option<&StepSection> {
        self.sections.iter().find(|s| s.number == step)
    }

    /// First error line in `range` (the whole log when `None`).
    pub fn first_error(&self, range: Option<Range<usize>>) -> Option<usize> {
        let range = range.unwrap_or(0..self.lines.len());
        let start = range.start;
        self.lines
            .get(range)?
            .iter()
            .position(|l| l.kind == LineKind::Error)
            .map(|ix| start + ix)
    }

    pub fn error_count(&self) -> usize {
        self.lines.iter().filter(|l| l.kind == LineKind::Error).count()
    }
}

/// Parses `raw`; `steps` (from the jobs API) assign lines to steps when given.
pub fn parse_log(raw: &str, steps: &[Step]) -> ParsedLog {
    let mut lines = Vec::new();
    let mut depth: u8 = 0;
    for raw_line in raw.trim_start_matches('\u{feff}').lines() {
        let (timestamp, rest) = split_timestamp(raw_line);
        let text = strip_ansi(rest);
        if text.starts_with("##[endgroup]") {
            depth = depth.saturating_sub(1);
            continue;
        }
        let (kind, text) = classify(&text);
        let line_depth = depth;
        if kind == LineKind::Group {
            depth = depth.saturating_add(1);
        }
        lines.push(LogLine {
            text,
            kind,
            depth: line_depth,
            timestamp,
            step: None,
        });
    }
    let sections = assign_steps(&mut lines, steps);
    ParsedLog { lines, sections }
}

/// `2026-11-25T10:00:00.1234567Z rest` → (timestamp, `rest`).
fn split_timestamp(line: &str) -> (Option<Millis>, &str) {
    let Some((head, rest)) = line.split_once(' ') else {
        return (parse_rfc3339(line).filter(|_| looks_like_timestamp(line)), "");
    };
    if !looks_like_timestamp(head) {
        return (None, line);
    }
    match parse_rfc3339(head) {
        Some(ts) => (Some(ts), rest),
        None => (None, line),
    }
}

fn looks_like_timestamp(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 20 && b.get(4) == Some(&b'-') && b.get(10) == Some(&b'T') && s.ends_with('Z')
}

/// Removes CSI (`ESC [ … final`) and OSC (`ESC ] … BEL | ESC \`) sequences.
pub fn strip_ansi(s: &str) -> String {
    if !s.contains('\u{1b}') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\u{7}' {
                        break;
                    }
                    if c == '\u{1b}' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

const MARKERS: &[(&str, LineKind)] = &[
    ("##[group]", LineKind::Group),
    ("##[error]", LineKind::Error),
    ("##[warning]", LineKind::Warning),
    ("##[notice]", LineKind::Notice),
    ("##[debug]", LineKind::Debug),
    ("##[command]", LineKind::Command),
    ("[command]", LineKind::Command),
];

fn classify(text: &str) -> (LineKind, String) {
    for (marker, kind) in MARKERS {
        if let Some(rest) = text.strip_prefix(marker) {
            return (*kind, rest.to_string());
        }
    }
    let kind = if looks_like_error(text) {
        LineKind::Error
    } else if looks_like_warning(text) {
        LineKind::Warning
    } else {
        LineKind::Normal
    };
    (kind, text.to_string())
}

/// Common shapes of a failure in tool output (test runners, compilers, npm, shells).
pub fn looks_like_error(text: &str) -> bool {
    let trimmed = text.trim_start();
    let lower_start = trimmed.get(..6).map(str::to_ascii_lowercase);
    lower_start.as_deref() == Some("error:")
        || trimmed.starts_with("error[")
        || trimmed.starts_with("ERROR")
        || trimmed.starts_with("npm ERR!")
        || text.contains("Error:")
        || has_word(text, "FAILED")
        || nonzero_exit_code(text)
}

fn looks_like_warning(text: &str) -> bool {
    let trimmed = text.trim_start();
    trimmed
        .get(..8)
        .is_some_and(|s| s.eq_ignore_ascii_case("warning:"))
        || trimmed.starts_with("WARN")
}

fn has_word(text: &str, word: &str) -> bool {
    text.match_indices(word).any(|(ix, _)| {
        let before = text.get(..ix).and_then(|s| s.chars().next_back());
        let after = text.get(ix + word.len()..).and_then(|s| s.chars().next());
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

fn nonzero_exit_code(text: &str) -> bool {
    text.match_indices("exit code ").any(|(ix, m)| {
        let digits: String = text
            .get(ix + m.len()..)
            .unwrap_or_default()
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        digits.parse::<u32>().is_ok_and(|code| code != 0)
    })
}

struct Window {
    number: u32,
    start: i64,
    end: Option<i64>,
}

/// Fills `LogLine::step` and returns the line range of each step.
fn assign_steps(lines: &mut [LogLine], steps: &[Step]) -> Vec<StepSection> {
    let mut windows: Vec<Window> = steps
        .iter()
        .filter_map(|s| {
            Some(Window {
                number: s.number,
                start: parse_rfc3339(s.started_at.as_deref()?)? / 1000,
                end: s
                    .completed_at
                    .as_deref()
                    .and_then(parse_rfc3339)
                    .map(|t| t / 1000),
            })
        })
        .collect();
    windows.sort_by_key(|w| w.number);
    if windows.is_empty() {
        return Vec::new();
    }
    let mut cursor = 0;
    for line in lines.iter_mut() {
        if let Some(ts) = line.timestamp {
            let second = ts / 1000;
            while let (Some(current), Some(next)) = (windows.get(cursor), windows.get(cursor + 1)) {
                let current_over = current.end.is_some_and(|end| end < second);
                let opens_next = line.kind == LineKind::Group && line.depth == 0;
                if next.start <= second && (next.start < second || current_over || opens_next) {
                    cursor += 1;
                } else {
                    break;
                }
            }
        }
        line.step = windows.get(cursor).map(|w| w.number);
    }
    let mut sections: Vec<StepSection> = Vec::new();
    for (ix, line) in lines.iter().enumerate() {
        let Some(number) = line.step else { continue };
        match sections.last_mut() {
            Some(last) if last.number == number => last.lines.end = ix + 1,
            _ => sections.push(StepSection {
                number,
                lines: ix..ix + 1,
            }),
        }
    }
    sections
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../tests/fixtures/job_log.txt");

    fn step(number: u32, name: &str, start: &str, end: &str, conclusion: &str) -> Step {
        Step {
            name: name.into(),
            status: "completed".into(),
            conclusion: Some(conclusion.into()),
            number,
            started_at: Some(format!("2026-11-25T10:00:{start}Z")),
            completed_at: Some(format!("2026-11-25T10:00:{end}Z")),
        }
    }

    fn steps() -> Vec<Step> {
        vec![
            step(1, "Set up job", "00", "02", "success"),
            step(2, "Checkout", "02", "03", "success"),
            step(3, "Run tests", "03", "09", "failure"),
            step(4, "Complete job", "09", "10", "success"),
        ]
    }

    #[test]
    fn strips_timestamps_ansi_and_markers() {
        let log = parse_log(FIXTURE, &[]);
        assert!(log.lines.iter().all(|l| !l.text.contains("2026-11-25T")));
        assert!(log.lines.iter().all(|l| !l.text.contains('\u{1b}')));
        assert!(log.lines.iter().all(|l| !l.text.starts_with("##[")));
        let group = log.lines.iter().find(|l| l.kind == LineKind::Group).unwrap();
        assert_eq!(group.text, "Operating System");
        assert!(
            log.lines
                .iter()
                .any(|l| l.text == "  1) User validates email" && l.kind == LineKind::Normal)
        );
    }

    #[test]
    fn tracks_group_depth() {
        let log = parse_log(FIXTURE, &[]);
        let header = log
            .lines
            .iter()
            .position(|l| l.text == "Operating System")
            .unwrap();
        assert_eq!(log.lines[header].depth, 0);
        assert_eq!(log.lines[header + 1].depth, 1);
        let after = log
            .lines
            .iter()
            .find(|l| l.text == "Runner Image Provisioner")
            .unwrap();
        assert_eq!(after.depth, 0);
    }

    #[test]
    fn detects_errors_and_warnings() {
        let log = parse_log(FIXTURE, &[]);
        let errors: Vec<&str> = log
            .lines
            .iter()
            .filter(|l| l.kind == LineKind::Error)
            .map(|l| l.text.as_str())
            .collect();
        assert_eq!(
            errors,
            [
                "Failure/Error: expect(user).to be_valid",
                "rspec ./spec/models/user_spec.rb:42 FAILED",
                "Process completed with exit code 1."
            ]
        );
        assert!(
            log.lines
                .iter()
                .any(|l| l.kind == LineKind::Warning && l.text.starts_with("Node.js 16"))
        );
        assert!(
            log.lines
                .iter()
                .any(|l| l.kind == LineKind::Command && l.text.starts_with("/usr/bin/git"))
        );
    }

    #[test]
    fn error_heuristics_avoid_false_positives() {
        assert!(looks_like_error("error: could not compile `app`"));
        assert!(looks_like_error("error[E0308]: mismatched types"));
        assert!(looks_like_error("npm ERR! code 1"));
        assert!(looks_like_error("make: *** Error 2 exit code 2"));
        assert!(!looks_like_error("0 errors, 3 warnings"));
        assert!(!looks_like_error("Process completed with exit code 0."));
        assert!(!looks_like_error("UNFAILEDX"));
        assert!(!looks_like_error("handles errors gracefully"));
    }

    #[test]
    fn assigns_lines_to_steps_by_time() {
        let log = parse_log(FIXTURE, &steps());
        let numbers: Vec<u32> = log.sections.iter().map(|s| s.number).collect();
        assert_eq!(numbers, [1, 2, 3, 4]);
        let tests = log.section(3).unwrap();
        let first = &log.lines[tests.lines.start];
        assert_eq!(first.text, "Run bundle exec rspec");
        let error = log.first_error(Some(tests.lines.clone())).unwrap();
        assert_eq!(log.lines[error].text, "Failure/Error: expect(user).to be_valid");
        let checkout = log.section(2).unwrap();
        assert_eq!(log.lines[checkout.lines.start].text, "Run actions/checkout@v4");
        assert_eq!(log.first_error(Some(checkout.lines.clone())), None);
        assert_eq!(
            log.lines[log.section(4).unwrap().lines.start].text,
            "Cleaning up orphan processes"
        );
    }

    #[test]
    fn without_steps_there_are_no_sections() {
        let log = parse_log(FIXTURE, &[]);
        assert!(log.sections.is_empty());
        assert_eq!(log.error_count(), 3);
    }

    #[test]
    fn strips_osc_and_csi_sequences() {
        assert_eq!(strip_ansi("\u{1b}[31;1mred\u{1b}[0m"), "red");
        assert_eq!(
            strip_ansi("\u{1b}]8;;http://x\u{7}link\u{1b}]8;;\u{1b}\\"),
            "link"
        );
        assert_eq!(strip_ansi("plain"), "plain");
    }
}
