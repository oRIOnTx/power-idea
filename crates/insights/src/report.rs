//! What the panel header shows for a period: moving-average score, delta to
//! the previous period of the same length, and block averages now vs then.

use super::SessionSummary;
use super::score::moving_average;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Period {
    Week,
    #[default]
    Month,
    Quarter,
    All,
}

impl Period {
    pub const ALL: [Period; 4] = [Period::Week, Period::Month, Period::Quarter, Period::All];

    pub fn label(self) -> &'static str {
        match self {
            Self::Week => "7d",
            Self::Month => "30d",
            Self::Quarter => "90d",
            Self::All => "All",
        }
    }

    pub fn days(self) -> Option<i64> {
        match self {
            Self::Week => Some(7),
            Self::Month => Some(30),
            Self::Quarter => Some(90),
            Self::All => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Report {
    /// Sessions in the period, oldest first.
    pub sessions: Vec<SessionSummary>,
    /// Moving average of the last 7 sessions in the period.
    pub score: Option<f64>,
    /// Period mean minus the previous period's mean (score points).
    pub delta: Option<f64>,
    pub blocks_now: Option<[f64; 7]>,
    pub blocks_before: Option<[f64; 7]>,
    pub total_cost: f64,
    pub total_tokens: u64,
}

fn mean_blocks(sessions: &[&SessionSummary]) -> Option<[f64; 7]> {
    (!sessions.is_empty()).then(|| {
        let mut sum = [0.0; 7];
        for s in sessions {
            for (acc, b) in sum.iter_mut().zip(s.blocks) {
                *acc += b;
            }
        }
        sum.map(|v| v / sessions.len() as f64)
    })
}

fn mean_score(sessions: &[&SessionSummary]) -> Option<f64> {
    (!sessions.is_empty()).then(|| sessions.iter().map(|s| s.score).sum::<f64>() / sessions.len() as f64)
}

/// `project` filters by working folder; `now` is Unix seconds.
pub fn report(all: &[SessionSummary], period: Period, project: Option<&str>, now: i64) -> Report {
    let in_project = |s: &&SessionSummary| project.is_none_or(|p| s.project == p);
    let (current, previous): (Vec<&SessionSummary>, Vec<&SessionSummary>) = match period.days() {
        None => (all.iter().filter(in_project).collect(), Vec::new()),
        Some(days) => {
            let span = days * 86_400;
            let from = now - span;
            let current = all
                .iter()
                .filter(in_project)
                .filter(|s| s.started >= from)
                .collect();
            let previous = all
                .iter()
                .filter(in_project)
                .filter(|s| s.started >= from - span && s.started < from)
                .collect();
            (current, previous)
        }
    };
    let scores: Vec<f64> = current.iter().map(|s| s.score).collect();
    Report {
        score: moving_average(&scores, 7),
        delta: mean_score(&current)
            .zip(mean_score(&previous))
            .map(|(now, before)| now - before),
        blocks_now: mean_blocks(&current),
        blocks_before: mean_blocks(&previous),
        total_cost: current.iter().map(|s| s.metrics.cost_usd).sum(),
        total_tokens: current.iter().map(|s| s.metrics.total_tokens).sum(),
        sessions: current.into_iter().cloned().collect(),
    }
}

/// Distinct projects, most recently used first.
pub fn projects(all: &[SessionSummary]) -> Vec<String> {
    let mut seen = Vec::new();
    for s in all.iter().rev() {
        if !s.project.is_empty() && !seen.contains(&s.project) {
            seen.push(s.project.clone());
        }
    }
    seen
}

/// "Today", "Yesterday", "3 days ago", then the UTC date (`2026-11-02`).
pub fn day_label(timestamp: i64, now: i64) -> String {
    const DAY: i64 = 86_400;
    match now.div_euclid(DAY) - timestamp.div_euclid(DAY) {
        ..=0 => "Today".into(),
        1 => "Yesterday".into(),
        days @ 2..=6 => format!("{days} days ago"),
        _ => {
            // Civil from days (Howard Hinnant).
            let z = timestamp.div_euclid(DAY) + 719_468;
            let era = z.div_euclid(146_097);
            let doe = z - era * 146_097;
            let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
            let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
            let mp = (5 * doy + 2) / 153;
            let day = doy - (153 * mp + 2) / 5 + 1;
            let month = if mp < 10 { mp + 3 } else { mp - 9 };
            let year = yoe + era * 400 + i64::from(month <= 2);
            format!("{year}-{month:02}-{day:02}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Period, day_label, projects, report};
    use crate::SessionSummary;

    const DAY: i64 = 86_400;

    fn session(project: &str, days_ago: i64, score: f64) -> SessionSummary {
        SessionSummary {
            project: project.into(),
            started: 100 * DAY - days_ago * DAY,
            score,
            blocks: [score; 7],
            ..SessionSummary::default()
        }
    }

    #[test]
    fn compares_periods_and_filters_projects() {
        let all = vec![
            session("/a", 10, 40.0),
            session("/b", 9, 60.0),
            session("/a", 3, 70.0),
            session("/a", 1, 90.0),
        ];
        let week = report(&all, Period::Week, None, 100 * DAY);
        assert_eq!(week.sessions.len(), 2);
        assert_eq!(week.score, Some(80.0));
        assert_eq!(week.delta, Some(30.0), "80 now vs 50 the week before");
        assert_eq!(week.blocks_before.map(|b| b[0]), Some(50.0));

        let only_a = report(&all, Period::Week, Some("/a"), 100 * DAY);
        assert_eq!(only_a.delta, Some(40.0));
        let everything = report(&all, Period::All, None, 100 * DAY);
        assert_eq!((everything.sessions.len(), everything.delta), (4, None));
        assert_eq!(report(&[], Period::Month, None, 0).score, None);
        assert_eq!(projects(&all), ["/a", "/b"]);
    }

    #[test]
    fn labels_days() {
        let now = crate::parse::parse_timestamp("2026-11-22T12:00:00Z").unwrap();
        assert_eq!(day_label(now - 60, now), "Today");
        assert_eq!(day_label(now - DAY, now), "Yesterday");
        assert_eq!(day_label(now - 3 * DAY, now), "3 days ago");
        assert_eq!(day_label(now - 20 * DAY, now), "2026-11-02");
        assert_eq!(
            day_label(
                crate::parse::parse_timestamp("2024-02-29T01:00:00Z").unwrap(),
                now
            ),
            "2024-02-29"
        );
    }
}
