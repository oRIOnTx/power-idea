//! Deterministic per-session metrics (no LLM): pure functions over a parsed session.

use serde::{Deserialize, Serialize};

use super::parse::Session;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Metrics {
    pub prompts: usize,
    pub corrections: usize,
    pub correction_rate: f64,
    pub tool_calls: usize,
    pub tool_errors: usize,
    pub error_rate: f64,
    /// Tool calls the agent made per prompt.
    pub steps_per_prompt: f64,
    pub interrupts: usize,
    /// Tests, linters or builds ran.
    pub verified: bool,
    pub committed: bool,
    /// Runs of the same tool call repeated 3+ times in a row.
    pub loops: usize,
    pub first_prompt_chars: usize,
    pub first_prompt_paths: bool,
    pub first_prompt_criteria: bool,
    /// Share of input served from the prompt cache.
    pub cache_hit: f64,
    pub total_tokens: u64,
    pub cost_usd: f64,
    pub duration_secs: i64,
    pub compactions: usize,
}

const VERIFY_WORDS: &[&str] = &[
    "test", "rspec", "pytest", "jest", "vitest", "clippy", "lint", "rubocop", "eslint", "tsc", "check",
    "build", "vet",
];

/// A shell command that verifies work (tests, linters, builds).
pub fn is_verification(command: &str) -> bool {
    let lower = command.to_lowercase();
    lower
        .split(|c: char| !c.is_alphanumeric() && c != '-')
        .any(|word| {
            VERIFY_WORDS
                .iter()
                .any(|v| word == *v || word.starts_with(&format!("{v}-")) || word.ends_with(&format!("-{v}")))
        })
}

pub fn is_commit(command: &str) -> bool {
    command.contains("git commit")
}

fn ratio(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 / whole as f64
    }
}

/// Runs of 3+ identical consecutive tool calls (the agent going in circles).
fn loops(session: &Session) -> usize {
    let mut count = 0;
    let mut run = 1;
    for pair in session.tools.windows(2) {
        let [prev, next] = pair else { continue };
        if prev.name == next.name && prev.input == next.input {
            run += 1;
            if run == 3 {
                count += 1;
            }
        } else {
            run = 1;
        }
    }
    count
}

pub fn metrics(session: &Session) -> Metrics {
    let prompts = session.prompts.len();
    let corrections = session.prompts.iter().filter(|p| p.correction).count();
    let tool_errors = session.tools.iter().filter(|t| t.error).count();
    let shell = || {
        session
            .tools
            .iter()
            .filter(|t| t.name == "Bash")
            .map(|t| t.input.as_str())
    };
    let first = session.prompts.first();
    let input_side = session.input_tokens + session.cache_read_tokens + session.cache_write_tokens;
    Metrics {
        prompts,
        corrections,
        correction_rate: ratio(corrections, prompts),
        tool_calls: session.tools.len(),
        tool_errors,
        error_rate: ratio(tool_errors, session.tools.len()),
        steps_per_prompt: if prompts == 0 {
            0.0
        } else {
            session.tools.len() as f64 / prompts as f64
        },
        interrupts: session.interrupts,
        verified: shell().any(is_verification),
        committed: shell().any(is_commit),
        loops: loops(session),
        first_prompt_chars: first.map_or(0, |p| p.chars),
        first_prompt_paths: first.is_some_and(|p| p.mentions_paths),
        first_prompt_criteria: first.is_some_and(|p| p.has_criteria),
        cache_hit: if input_side == 0 {
            0.0
        } else {
            session.cache_read_tokens as f64 / input_side as f64
        },
        total_tokens: session.total_tokens(),
        cost_usd: session.cost_usd,
        duration_secs: session.duration_secs(),
        compactions: session.compactions,
    }
}

#[cfg(test)]
mod tests {
    use super::{is_verification, metrics};
    use crate::parse::{parse_session, tests::fixture_lines};

    #[test]
    fn computes_metrics_from_the_fixture() {
        let m = metrics(&parse_session(fixture_lines()).unwrap());
        assert_eq!(
            (m.prompts, m.corrections, m.tool_calls, m.tool_errors),
            (2, 1, 3, 1)
        );
        assert!((m.correction_rate - 0.5).abs() < 1e-9);
        assert!(m.verified && m.committed);
        assert!(m.first_prompt_paths && m.first_prompt_criteria);
        assert_eq!((m.interrupts, m.compactions, m.loops), (1, 1, 0));
        assert!(m.cache_hit > 0.9);
    }

    #[test]
    fn recognises_verification_commands() {
        for cmd in [
            "cargo test -p app",
            "bundle exec rspec",
            "npm run lint",
            "scripts/check.sh",
            "go vet ./...",
            "make build",
        ] {
            assert!(is_verification(cmd), "{cmd}");
        }
        for cmd in ["git status", "ls -la", "cat latest.txt"] {
            assert!(!is_verification(cmd), "{cmd}");
        }
    }
}
