//! Scores: each of seven blocks maps metrics to 0–100; the Session Score is
//! their weighted mean. Adding a block = one `Block` variant + one arm.

use serde::{Deserialize, Serialize};

use super::metrics::Metrics;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Block {
    Efficiency,
    Autonomy,
    Corrections,
    PromptQuality,
    Verification,
    ToolErrors,
    ContextHygiene,
}

impl Block {
    pub const ALL: [Block; 7] = [
        Block::Efficiency,
        Block::Autonomy,
        Block::Corrections,
        Block::PromptQuality,
        Block::Verification,
        Block::ToolErrors,
        Block::ContextHygiene,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Efficiency => "Efficiency",
            Self::Autonomy => "Autonomy",
            Self::Corrections => "Corrections",
            Self::PromptQuality => "Prompt quality",
            Self::Verification => "Verification",
            Self::ToolErrors => "Tool errors",
            Self::ContextHygiene => "Context hygiene",
        }
    }

    pub fn score(self, m: &Metrics) -> f64 {
        let value = match self {
            // Cache hits mean the agent reused context instead of re-reading it.
            Self::Efficiency => m.cache_hit * 100.0,
            Self::Autonomy => 20.0 + m.steps_per_prompt.min(8.0) * 10.0 - m.interrupts as f64 * 10.0,
            Self::Corrections => 100.0 - m.correction_rate * 200.0,
            Self::PromptQuality => {
                25.0 + 25.0 * f64::from(u8::from(m.first_prompt_paths))
                    + 25.0 * f64::from(u8::from(m.first_prompt_criteria))
                    + 25.0 * f64::from(u8::from(m.first_prompt_chars >= 120))
            }
            // Q&A sessions without tools have nothing to verify.
            Self::Verification if m.tool_calls == 0 => 70.0,
            Self::Verification => {
                70.0 * f64::from(u8::from(m.verified)) + 30.0 * f64::from(u8::from(m.committed))
            }
            Self::ToolErrors => 100.0 - m.error_rate * 250.0 - m.loops as f64 * 15.0,
            // Up to 2M tokens is fine; at 20M the session is far too long.
            Self::ContextHygiene => {
                let millions = m.total_tokens as f64 / 1_000_000.0;
                100.0 - ((millions - 2.0).max(0.0) / 18.0 * 100.0) - m.compactions as f64 * 15.0
            }
        };
        value.clamp(0.0, 100.0)
    }
}

/// Block weights, in `Block::ALL` order.
pub type Weights = [f64; 7];
pub const DEFAULT_WEIGHTS: Weights = [1.0; 7];

pub fn blocks(m: &Metrics) -> [f64; 7] {
    Block::ALL.map(|block| block.score(m))
}

pub fn session_score(blocks: &[f64; 7], weights: &Weights) -> f64 {
    let total: f64 = weights.iter().sum();
    if total <= 0.0 {
        return 0.0;
    }
    blocks.iter().zip(weights).map(|(b, w)| b * w).sum::<f64>() / total
}

/// Mean of the last `n` values (a moving average for the header and trend).
pub fn moving_average(values: &[f64], n: usize) -> Option<f64> {
    let window = &values[values.len().saturating_sub(n)..];
    (!window.is_empty()).then(|| window.iter().sum::<f64>() / window.len() as f64)
}

#[cfg(test)]
mod tests {
    use super::{Block, DEFAULT_WEIGHTS, blocks, moving_average, session_score};
    use crate::metrics::{Metrics, metrics};
    use crate::parse::{parse_session, tests::fixture_lines};

    #[test]
    fn scores_blocks_and_sessions() {
        let m = metrics(&parse_session(fixture_lines()).unwrap());
        let b = blocks(&m);
        assert_eq!(b[2], 0.0, "half the prompts were corrections");
        assert_eq!(b[4], 100.0, "verified and committed");
        let score = session_score(&b, &DEFAULT_WEIGHTS);
        assert!((0.0..=100.0).contains(&score));

        let quiet = Metrics {
            cache_hit: 0.5,
            ..Metrics::default()
        };
        assert_eq!(Block::Verification.score(&quiet), 70.0);
        assert_eq!(Block::Efficiency.score(&quiet), 50.0);
        let huge = Metrics {
            total_tokens: 40_000_000,
            ..Metrics::default()
        };
        assert_eq!(Block::ContextHygiene.score(&huge), 0.0);
    }

    #[test]
    fn weighs_and_averages() {
        let only_first = [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        assert_eq!(
            session_score(&[80.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], &only_first),
            80.0
        );
        assert_eq!(moving_average(&[10.0, 20.0, 30.0], 2), Some(25.0));
        assert_eq!(moving_average(&[], 7), None);
    }
}
