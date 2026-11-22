//! Claude Session Insights: how effectively the user steers Claude Code,
//! from the local transcripts in `~/.claude/projects`. Layers: `parse`
//! (transcripts → facts) → `metrics` → `score` → `rules` (insights), with
//! `report` (periods and trends), `store` (SQLite) and `index` (incremental, off the UI thread). Everything
//! stays on this machine.

pub mod metrics;
pub mod parse;
pub mod report;
pub mod rules;
pub mod score;
pub mod store;

use std::path::Path;
use std::time::UNIX_EPOCH;

use anyhow::Result;
use serde::{Deserialize, Serialize};

pub use metrics::Metrics;
pub use report::{Period, Report, day_label, projects, report};
pub use rules::Insight;
pub use score::{Block, DEFAULT_WEIGHTS, Weights};
pub use store::Store;

/// One indexed session as the panel shows it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub project: String,
    pub started: i64,
    pub ended: i64,
    pub model: String,
    pub score: f64,
    pub blocks: [f64; 7],
    pub metrics: Metrics,
}

/// Parses, measures and scores one transcript.
pub fn summarize(lines: impl Iterator<Item = String>, weights: &Weights) -> Option<SessionSummary> {
    let session = parse::parse_session(lines)?;
    let metrics = metrics::metrics(&session);
    let blocks = score::blocks(&metrics);
    Some(SessionSummary {
        score: score::session_score(&blocks, weights),
        id: session.id,
        project: session.project,
        started: session.started,
        ended: session.ended,
        model: session.model,
        blocks,
        metrics,
    })
}

/// Indexes new or changed transcripts under `claude_dir/projects`; returns how many were (re)read.
pub fn index(store: &Store, claude_dir: &Path, weights: &Weights) -> Result<usize> {
    let mut indexed = 0;
    let Ok(projects) = std::fs::read_dir(claude_dir.join("projects")) else {
        return Ok(0);
    };
    for project in projects.flatten() {
        let Ok(files) = std::fs::read_dir(project.path()) else {
            continue;
        };
        for file in files.flatten() {
            let path = file.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let Ok(meta) = file.metadata() else { continue };
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs() as i64);
            let key = path.to_string_lossy().into_owned();
            if !store.file_changed(&key, mtime, meta.len() as i64)? {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            if let Some(summary) = summarize(text.lines().map(str::to_string), weights) {
                store.upsert(&summary)?;
            }
            store.mark_file(&key, mtime, meta.len() as i64)?;
            indexed += 1;
        }
    }
    Ok(indexed)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{DEFAULT_WEIGHTS, Store, index};
    use crate::parse::tests::FIXTURE;

    #[test]
    fn indexes_incrementally_and_clears() {
        let dir = std::env::temp_dir().join(format!("pi-insights-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("claude/projects/-p-shop")).unwrap();
        fs::write(dir.join("claude/projects/-p-shop/s1.jsonl"), FIXTURE).unwrap();
        fs::write(dir.join("claude/projects/-p-shop/notes.txt"), "ignored").unwrap();

        let store = Store::open(&dir.join("insights.sqlite")).unwrap();
        assert_eq!(index(&store, &dir.join("claude"), &DEFAULT_WEIGHTS).unwrap(), 1);
        assert_eq!(
            index(&store, &dir.join("claude"), &DEFAULT_WEIGHTS).unwrap(),
            0,
            "unchanged file is skipped"
        );
        let sessions = store.sessions().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].metrics.prompts, 2);

        store.dismiss("corrections").unwrap();
        assert_eq!(store.dismissed().unwrap(), ["corrections"]);
        store.clear().unwrap();
        assert!(store.sessions().unwrap().is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
}
