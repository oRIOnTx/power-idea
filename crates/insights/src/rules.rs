//! Rule-based insights: each rule looks at recent sessions and, when it
//! fires, says what the data shows, why it matters and what to try. A new
//! rule is one function added to `RULES`. `GENERAL` tips (good practice, no
//! data needed) fill the remaining slots, so the panel always has advice.

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
pub const RULES: &[Rule] = &[
    corrections,
    verification,
    long_sessions,
    tool_errors,
    interrupts,
    vague_prompts,
    loops,
    compactions,
];

/// Good-practice tips shown after the data-driven ones, in this order.
pub const GENERAL: &[Insight] = &[
    general(
        "plan-first",
        "Plan bigger changes first",
        "A plan you approve up front is cheaper than undoing edits that went the wrong way.",
        "For anything touching several files, use plan mode (Shift+Tab) and review the plan.",
    ),
    general(
        "claude-md",
        "Keep CLAUDE.md current",
        "The agent reads it every session; stale or missing rules get rediscovered by trial and error.",
        "When you correct the same thing twice, add the rule to CLAUDE.md (or say \"remember this\").",
    ),
    general(
        "show-dont-tell",
        "Give the agent the evidence",
        "Exact errors, failing output and screenshots remove guessing about what is wrong.",
        "Paste the full error or a screenshot instead of describing it.",
    ),
    general(
        "one-task",
        "One task per session",
        "Unrelated tasks in one context make the agent mix them up and cost more tokens.",
        "Finish a task, commit, then /clear before the next one.",
    ),
];

const fn general(
    id: &'static str,
    title: &'static str,
    why: &'static str,
    try_this: &'static str,
) -> Insight {
    Insight {
        id,
        title,
        evidence: String::new(),
        why,
        try_this,
    }
}

fn last(sessions: &[SessionSummary], n: usize) -> &[SessionSummary] {
    sessions
        .get(sessions.len().saturating_sub(n)..)
        .unwrap_or_default()
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
    let threshold = *tokens.get(tokens.len() * 3 / 4)?;
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

fn vague_prompts(sessions: &[SessionSummary]) -> Option<Insight> {
    let recent = last(sessions, 10);
    let vague = recent
        .iter()
        .filter(|s| !s.metrics.first_prompt_paths && !s.metrics.first_prompt_criteria)
        .count();
    (recent.len() >= 3 && vague as f64 > recent.len() as f64 * 0.6).then(|| Insight {
        id: "vague-prompts",
        title: "First prompts are vague",
        evidence: format!(
            "{vague} of the last {} sessions started without files or a goal to check against.",
            recent.len()
        ),
        why: "Without a target the agent explores first, and you correct it later.",
        try_this: "Name the files or area involved and how to tell the task is done.",
    })
}

fn loops(sessions: &[SessionSummary]) -> Option<Insight> {
    let recent = last(sessions, 10);
    let total: usize = recent.iter().map(|s| s.metrics.loops).sum();
    (recent.len() >= 3 && total >= 3).then(|| Insight {
        id: "loops",
        title: "The agent goes in circles",
        evidence: format!(
            "{total} runs of the same tool call repeated 3+ times in the last {} sessions.",
            recent.len()
        ),
        why: "Repeating a call means the agent is stuck and hopes for a different result.",
        try_this: "When you see a repeat, stop it (Esc) and give the missing fact or a different approach.",
    })
}

fn compactions(sessions: &[SessionSummary]) -> Option<Insight> {
    let recent = last(sessions, 10);
    let compacted = recent.iter().filter(|s| s.metrics.compactions > 0).count();
    (recent.len() >= 3 && compacted * 3 >= recent.len()).then(|| Insight {
        id: "compactions",
        title: "Sessions outgrow the context",
        evidence: format!(
            "{compacted} of the last {} sessions were compacted.",
            recent.len()
        ),
        why: "Compaction summarises the conversation and drops details the agent then has to rediscover.",
        try_this: "Split big work into steps with a short written plan, and start each step fresh.",
    })
}

/// Insights that fire, skipping dismissed ones, best first; general tips fill the rest.
pub fn insights(sessions: &[SessionSummary], dismissed: &[String], limit: usize) -> Vec<Insight> {
    RULES
        .iter()
        .filter_map(|rule| rule(sessions))
        .chain(GENERAL.iter().cloned())
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
        assert_eq!(
            ids,
            [
                "corrections",
                "verification",
                "long-sessions",
                "vague-prompts",
                "plan-first"
            ]
        );
        let ids: Vec<&str> = insights(&sessions, &["corrections".into()], 2)
            .iter()
            .map(|i| i.id)
            .collect();
        assert_eq!(ids, ["verification", "long-sessions"]);

        // Good sessions get only general advice.
        let mut good: Vec<_> = (0..8).map(|_| session(0.0, true, 1_000, 80.0)).collect();
        for s in &mut good {
            s.metrics.first_prompt_criteria = true;
        }
        let tips = insights(&good, &["plan-first".into()], 3);
        assert!(tips.iter().all(|t| t.evidence.is_empty()));
        assert_eq!(tips[0].id, "claude-md");
    }
}
