//! Personal advice from Claude: recent metrics, fired rules and sample prompts
//! are put into one request to the local `claude` CLI. Runs only when the user
//! asks for it; nothing else in this crate leaves the machine.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, bail};

use super::SessionSummary;
use super::parse::prompt_texts;
use super::rules::RULES;

/// Sessions the request describes (the most recent ones).
pub const SESSIONS: usize = 10;
/// Prompts quoted per session, and how much of each.
const PROMPTS_PER_SESSION: usize = 3;
const PROMPT_CHARS: usize = 400;

/// What the user typed in one session, for the request.
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    pub project: String,
    pub prompts: Vec<String>,
}

fn last(sessions: &[SessionSummary]) -> &[SessionSummary] {
    &sessions[sessions.len().saturating_sub(SESSIONS)..]
}

fn truncate(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= PROMPT_CHARS {
        return text.to_string();
    }
    let cut: String = text.chars().take(PROMPT_CHARS).collect();
    format!("{cut}…")
}

/// The transcript of session `id`: `<claude_dir>/projects/<any>/<id>.jsonl`.
fn transcript(claude_dir: &Path, id: &str) -> Option<PathBuf> {
    std::fs::read_dir(claude_dir.join("projects"))
        .ok()?
        .flatten()
        .map(|project| project.path().join(format!("{id}.jsonl")))
        .find(|path| path.is_file())
}

/// The first prompts of the recent sessions, read from their transcripts.
pub fn samples(claude_dir: &Path, sessions: &[SessionSummary]) -> Vec<Sample> {
    last(sessions)
        .iter()
        .filter_map(|session| {
            let text = std::fs::read_to_string(transcript(claude_dir, &session.id)?).ok()?;
            let prompts: Vec<String> = prompt_texts(text.lines().map(str::to_string))
                .iter()
                .take(PROMPTS_PER_SESSION)
                .map(|p| truncate(p))
                .collect();
            let project = session.project.rsplit('/').next().unwrap_or_default().to_string();
            (!prompts.is_empty()).then_some(Sample { project, prompts })
        })
        .collect()
}

/// The request: instructions, per-session numbers, fired rules and sample prompts.
pub fn request(sessions: &[SessionSummary], samples: &[Sample]) -> String {
    let recent = last(sessions);
    let mut out = String::from(
        "You are a coach for a developer who works with Claude Code (an AI coding agent). \
         Below are measurements of their recent sessions and the prompts they wrote. \
         Give 3-5 specific, actionable recommendations to make their collaboration with Claude \
         more effective: how they phrase tasks, give context, verify results, manage sessions. \
         Ground every recommendation in the data (quote a prompt or a number) and show a better \
         phrasing where it helps. Answer in the language the developer writes their prompts in. \
         Plain text, a numbered list, no Markdown headings or tables, under 300 words.\n\n",
    );
    out.push_str("Recent sessions (oldest first):\n");
    for s in recent {
        let m = &s.metrics;
        out.push_str(&format!(
            "- score {:.0}/100, {} prompts ({} corrections), {} tool calls ({} failed), \
             {} interrupts, {} loops, {} compactions, verified: {}, committed: {}, \
             {} min, {}k tokens\n",
            s.score,
            m.prompts,
            m.corrections,
            m.tool_calls,
            m.tool_errors,
            m.interrupts,
            m.loops,
            m.compactions,
            if m.verified { "yes" } else { "no" },
            if m.committed { "yes" } else { "no" },
            m.duration_secs / 60,
            m.total_tokens / 1000,
        ));
    }
    let fired: Vec<String> = RULES
        .iter()
        .filter_map(|rule| rule(sessions))
        .map(|i| format!("- {}: {}", i.title, i.evidence))
        .collect();
    if !fired.is_empty() {
        out.push_str("\nPatterns already detected:\n");
        out.push_str(&fired.join("\n"));
        out.push('\n');
    }
    if !samples.is_empty() {
        out.push_str("\nTheir first prompts per session:\n");
        for sample in samples {
            out.push_str(&format!("[{}]\n", sample.project));
            for prompt in &sample.prompts {
                out.push_str(&format!("> {}\n", prompt.replace('\n', "\n> ")));
            }
        }
    }
    out
}

/// Asks the local `claude` CLI (no tools, nothing saved as a session) and returns its answer.
pub fn ask_claude(request: &str) -> Result<String> {
    let mut child = Command::new("claude")
        .args([
            "-p",
            "--no-session-persistence",
            "--tools",
            "",
            "--output-format",
            "text",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("could not start `claude` (is Claude Code installed and on PATH?)")?;
    child
        .stdin
        .take()
        .context("no stdin for claude")?
        .write_all(request.as_bytes())
        .context("sending the request to claude")?;
    let output = child.wait_with_output().context("waiting for claude")?;
    let answer = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() || answer.is_empty() {
        let err = String::from_utf8_lossy(&output.stderr);
        bail!("claude failed: {}", err.trim());
    }
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{request, samples, truncate};
    use crate::parse::tests::FIXTURE;
    use crate::{DEFAULT_WEIGHTS, summarize};

    #[test]
    fn builds_the_request_from_transcripts() {
        let dir = std::env::temp_dir().join(format!("pi-advice-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("projects/-p-shop")).unwrap();
        fs::write(dir.join("projects/-p-shop/s1.jsonl"), FIXTURE).unwrap();

        let session = summarize(FIXTURE.lines().map(str::to_string), &DEFAULT_WEIGHTS).unwrap();
        let samples = samples(&dir, std::slice::from_ref(&session));
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].project, "shop");
        assert_eq!(samples[0].prompts[1], "no, revert that commit");

        let text = request(&[session], &samples);
        assert!(text.contains("2 prompts (1 corrections)"));
        assert!(text.contains("[shop]\n> Fix the login bug"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn truncates_long_prompts() {
        assert_eq!(truncate("  short \n"), "short");
        assert_eq!(truncate(&"a".repeat(500)).chars().count(), 401);
    }
}
