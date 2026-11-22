//! Local SQLite store of indexed sessions (`insights.sqlite` in the IDE's
//! config folder): which transcript files were read, and per-session
//! metrics and scores. Raw transcripts are never copied.

use std::path::Path;

use anyhow::{Context as _, Result};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row as _, SqlitePool};

use super::SessionSummary;

pub struct Store {
    pool: SqlitePool,
    runtime: tokio::runtime::Runtime,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS files (path TEXT PRIMARY KEY, mtime INTEGER NOT NULL, size INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY, project TEXT NOT NULL, started INTEGER NOT NULL, ended INTEGER NOT NULL,
    model TEXT NOT NULL, score REAL NOT NULL, blocks TEXT NOT NULL, metrics TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS sessions_started ON sessions (started);
CREATE TABLE IF NOT EXISTS dismissed (id TEXT PRIMARY KEY);
";

impl Store {
    /// Opens (or creates) the database. Blocks: call from a background thread.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let options = SqliteConnectOptions::new().filename(path).create_if_missing(true);
        let pool = runtime
            .block_on(SqlitePoolOptions::new().max_connections(1).connect_with(options))
            .with_context(|| format!("opening {}", path.display()))?;
        runtime.block_on(sqlx::raw_sql(SCHEMA).execute(&pool))?;
        Ok(Self { pool, runtime })
    }

    /// Whether a transcript changed since it was last indexed.
    pub fn file_changed(&self, path: &str, mtime: i64, size: i64) -> Result<bool> {
        let row = self.runtime.block_on(
            sqlx::query("SELECT mtime, size FROM files WHERE path = ?")
                .bind(path)
                .fetch_optional(&self.pool),
        )?;
        Ok(row.is_none_or(|r| r.get::<i64, _>(0) != mtime || r.get::<i64, _>(1) != size))
    }

    pub fn mark_file(&self, path: &str, mtime: i64, size: i64) -> Result<()> {
        self.runtime.block_on(
            sqlx::query("INSERT OR REPLACE INTO files (path, mtime, size) VALUES (?, ?, ?)")
                .bind(path)
                .bind(mtime)
                .bind(size)
                .execute(&self.pool),
        )?;
        Ok(())
    }

    pub fn upsert(&self, s: &SessionSummary) -> Result<()> {
        self.runtime.block_on(
            sqlx::query(
                "INSERT OR REPLACE INTO sessions (id, project, started, ended, model, score, blocks, metrics)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&s.id)
            .bind(&s.project)
            .bind(s.started)
            .bind(s.ended)
            .bind(&s.model)
            .bind(s.score)
            .bind(serde_json::to_string(&s.blocks)?)
            .bind(serde_json::to_string(&s.metrics)?)
            .execute(&self.pool),
        )?;
        Ok(())
    }

    /// All sessions, oldest first.
    pub fn sessions(&self) -> Result<Vec<SessionSummary>> {
        let rows = self.runtime.block_on(
            sqlx::query("SELECT id, project, started, ended, model, score, blocks, metrics FROM sessions ORDER BY started")
                .fetch_all(&self.pool),
        )?;
        rows.into_iter()
            .map(|r| {
                Ok(SessionSummary {
                    id: r.get(0),
                    project: r.get(1),
                    started: r.get(2),
                    ended: r.get(3),
                    model: r.get(4),
                    score: r.get(5),
                    blocks: serde_json::from_str(&r.get::<String, _>(6))?,
                    metrics: serde_json::from_str(&r.get::<String, _>(7))?,
                })
            })
            .collect()
    }

    pub fn dismissed(&self) -> Result<Vec<String>> {
        let rows = self
            .runtime
            .block_on(sqlx::query("SELECT id FROM dismissed").fetch_all(&self.pool))?;
        Ok(rows.into_iter().map(|r| r.get(0)).collect())
    }

    pub fn dismiss(&self, id: &str) -> Result<()> {
        self.runtime.block_on(
            sqlx::query("INSERT OR IGNORE INTO dismissed (id) VALUES (?)")
                .bind(id)
                .execute(&self.pool),
        )?;
        Ok(())
    }

    /// "Clear all analytics data".
    pub fn clear(&self) -> Result<()> {
        self.runtime.block_on(
            sqlx::raw_sql("DELETE FROM files; DELETE FROM sessions; DELETE FROM dismissed;")
                .execute(&self.pool),
        )?;
        Ok(())
    }
}
