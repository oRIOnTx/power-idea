//! Rule-based insights: each rule looks at recent sessions and, when it
//! fires, says what the data shows, why it matters and what to try. A new
//! rule is one function added to `RULES`.

use super::SessionSummary;
use super::score::moving_average;

#[derive(Debug, Clone, PartialEq)]
pub struct Insight {
    /// Stable id, for dismissing.
    pub id: &'static str,
    pub title: &'static str,
    /// What the numbers show.
    pub evidence: String,
    pub why: &'static str,
    pub try_this: &'static str,
}

type Rule = fn(&[SessionSummary]) -> Option<Insight>;

/// Rules in priority order; the panel shows the first three that fire.
pub const RULES: &[Rule] = &[corrections, verification, long_sessions, tool_errors, interrupts];

fn last(sessions: &[SessionSummary], n: usize) -> &[SessionSummary] {
    &sessions[sessions.len().saturating_sub(n)..]
}

fn mean(values: impl Iterator<Item = f64>) -> Option<f64> {
    let values: Vec<f64> = values.collect();
    moving_average(&values, values.len())
}

fn corrections(sessions: &[SessionSummary]) -> Option<Insight> {
    let recent = last(sessions, 5);
    let rate = mean(recent.iter().map(|s| s.metrics.correction_rate))?;
    (recent.len() >= 3 && rate > 0.3).then(|| Insight {
        id: "corrections",
        title: "Prompts often need correcting",
        evidence: format!(
            "{:.0}% of your messages in the last {} sessions were corrections.",
            rate * 100.0,
            recent.len()
        ),
        why: "Each correction costs a round trip and usually means the first prompt left too much to guess.",
        try_this: "Start with what \"done\" means (tests to pass, files to touch) and the constraints.",
    })
}

fn verification(sessions: &[SessionSummary]) -> Option<Insight> {
    let with_tools: Vec<&SessionSummary> = last(sessions, 10)
        .iter()
        .filter(|s| s.metrics.tool_calls > 0)
        .collect();
    let verified = with_tools.iter().filter(|s| s.metrics.verified).count();
    (with_tools.len() >= 3 && (verified as f64) < with_tools.len() as f64 * 0.3).then(|| Insight {
        id: "verification",
        title: "Work is rarely verified",
        evidence: format!(
            "Tests, linters or builds ran in only {verified} of {} recent sessions.",
            with_tools.len()
        ),
        why: "Unverified changes come back as bugs and corrections later.",
        try_this: "Ask the agent to run the tests (or your check script) before it says it is done.",
    })
}

fn long_sessions(sessions: &[SessionSummary]) -> Option<Insight> {
    let recent = last(sessions, 30);
    if recent.len() < 6 {
        return None;
    }
    let mut tokens: Vec<u64> = recent.iter().map(|s| s.metrics.total_tokens).collect();
    tokens.sort_unstable();
    let threshold = tokens[tokens.len() * 3 / 4];
    let long = mean(
        recent
            .iter()
            .filter(|s| s.metrics.total_tokens >= threshold)
            .map(|s| s.score),
    )?;
    let rest = mean(
        recent
            .iter()
            .filter(|s| s.metrics.total_tokens < threshold)
            .map(|s| s.score),
    )?;
    (rest > 0.0 && long < rest * 0.85).then(|| Insight {
        id: "long-sessions",
        title: "Long sessions score lower",
        evidence: format!(
            "Your longest sessions score {:.0}% lower than the rest.",
            (1.0 - long / rest) * 100.0
        ),
        why: "A long context dilutes the agent's attention and drags old tasks into new ones.",
        try_this: "Start a fresh session (/clear) when you switch to a new task.",
    })
}

fn tool_errors(sessions: &[SessionSummary]) -> Option<Insight> {
    let recent = last(sessions, 10);
    let rate = mean(recent.iter().map(|s| s.metrics.error_rate))?;
    (recent.len() >= 3 && rate > 0.2).then(|| Insight {
        id: "tool-errors",
        title: "Many tool calls fail",
        evidence: format!("{:.0}% of tool calls failed in recent sessions.", rate * 100.0),
        why: "Failing calls burn tokens and often mean the agent is guessing about the environment.",
        try_this: "Tell the agent how to run and test the project (a CLAUDE.md with commands helps).",
    })
}

fn interrupts(sessions: &[SessionSummary]) -> Option<Insight> {
    let recent = last(sessions, 10);
    let total: usize = recent.iter().map(|s| s.metrics.interrupts).sum();
    (recent.len() >= 3 && total as f64 / recent.len() as f64 >= 1.5).then(|| Insight {
        id: "interrupts",
        title: "You often stop the agent mid-way",
        evidence: format!("{total} interruptions in the last {} sessions.", recent.len()),
        why: "Interruptions usually mean the agent went in the wrong direction early.",
        try_this: "Ask for a short plan first and approve it before the agent starts editing.",
    })
}

/// Insights that fire, skipping dismissed ones, best first.
pub fn insights(sessions: &[SessionSummary], dismissed: &[String], limit: usize) -> Vec<Insight> {
    RULES
        .iter()
        .filter_map(|rule| rule(sessions))
        .filter(|insight| !dismissed.iter().any(|d| d == insight.id))
        .take(limit)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::insights;
    use crate::SessionSummary;
    use crate::metrics::Metrics;

    fn session(correction_rate: f64, verified: bool, tokens: u64, score: f64) -> SessionSummary {
        SessionSummary {
            score,
            metrics: Metrics {
                correction_rate,
                verified,
                tool_calls: 5,
                total_tokens: tokens,
                ..Metrics::default()
            },
            ..SessionSummary::default()
        }
    }

    #[test]
    fn fires_rules_on_patterns() {
        let sessions: Vec<_> = (0..8)
            .map(|i| session(0.5, false, 1_000 * (i + 1), if i >= 6 { 30.0 } else { 70.0 }))
            .collect();
        let ids: Vec<&str> = insights(&sessions, &[], 5).iter().map(|i| i.id).collect();
        assert_eq!(ids, ["corrections", "verification", "long-sessions"]);
        let ids: Vec<&str> = insights(&sessions, &["corrections".into()], 2)
            .iter()
            .map(|i| i.id)
            .collect();
        assert_eq!(ids, ["verification", "long-sessions"]);

        let good: Vec<_> = (0..8).map(|_| session(0.0, true, 1_000, 80.0)).collect();
        assert!(insights(&good, &[], 3).is_empty());
    }
}
