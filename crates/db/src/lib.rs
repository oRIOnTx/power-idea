//! Database access for the IDE: PostgreSQL, MySQL/MariaDB and SQLite via
//! `sqlx`, Redis via `redis`. Blocking API on top of a shared tokio runtime,
//! meant to be called from background threads.

pub mod edit;
mod export;
mod keychain;
mod redis_db;
mod sql;

use std::sync::OnceLock;
use std::time::Duration;

use anyhow::Result;
use serde::{Deserialize, Serialize};

pub use export::to_csv;
pub use keychain::{delete_password, load_password, store_password};

/// Rows fetched per result set before the result is marked truncated.
pub const DEFAULT_ROW_LIMIT: usize = 500;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DbKind {
    Postgres,
    MySql,
    Sqlite,
    Redis,
}

impl DbKind {
    pub const ALL: [DbKind; 4] = [DbKind::Postgres, DbKind::MySql, DbKind::Sqlite, DbKind::Redis];

    pub fn label(self) -> &'static str {
        match self {
            Self::Postgres => "PostgreSQL",
            Self::MySql => "MySQL / MariaDB",
            Self::Sqlite => "SQLite",
            Self::Redis => "Redis",
        }
    }

    pub fn default_port(self) -> u16 {
        match self {
            Self::Postgres => 5432,
            Self::MySql => 3306,
            Self::Sqlite => 0,
            Self::Redis => 6379,
        }
    }

    pub fn is_sql(self) -> bool {
        self != Self::Redis
    }
}

/// A saved data source. The password lives in the macOS Keychain, keyed by `id`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ConnectionConfig {
    pub id: String,
    pub name: String,
    pub kind: DbKind,
    pub host: String,
    pub port: u16,
    pub user: String,
    /// Database name (SQL) or database index (Redis).
    pub database: String,
    /// SQLite database file.
    pub file: String,
    /// Require TLS (PostgreSQL / MySQL; `rediss://` for Redis).
    pub tls: bool,
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            kind: DbKind::Postgres,
            host: "localhost".into(),
            port: DbKind::Postgres.default_port(),
            user: String::new(),
            database: String::new(),
            file: String::new(),
            tls: false,
        }
    }
}

impl ConnectionConfig {
    /// Short description for lists, e.g. `postgres@localhost:5432/app`.
    pub fn summary(&self) -> String {
        if self.kind == DbKind::Sqlite {
            self.file.clone()
        } else {
            let user = if self.user.is_empty() {
                String::new()
            } else {
                format!("{}@", self.user)
            };
            let db = if self.database.is_empty() {
                String::new()
            } else {
                format!("/{}", self.database)
            };
            format!("{user}{}:{}{db}", self.host, self.port)
        }
    }
}

/// One result set of a query, already formatted for display.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResultSet {
    pub columns: Vec<String>,
    /// `None` is SQL NULL.
    pub rows: Vec<Vec<Option<String>>>,
    /// More rows were available than were fetched.
    pub truncated: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct QueryOutcome {
    pub sets: Vec<ResultSet>,
    pub rows_affected: u64,
    pub elapsed: Duration,
}

/// Database objects for the schema tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaNode {
    pub name: String,
    pub kind: NodeKind,
    pub children: Vec<SchemaNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    Schema,
    Table,
    View,
    Column { data_type: String },
    Key { key_type: String },
}

/// An open connection (pool for SQL, single connection for Redis).
pub enum Database {
    Sql(sql::SqlDatabase),
    Redis(redis_db::RedisDatabase),
}

impl Database {
    /// Connects and verifies the server answers.
    pub fn connect(config: &ConnectionConfig, password: Option<&str>) -> Result<Self> {
        Ok(match config.kind {
            DbKind::Redis => Self::Redis(redis_db::RedisDatabase::connect(config, password)?),
            _ => Self::Sql(sql::SqlDatabase::connect(config, password)?),
        })
    }

    /// Runs SQL (one or more statements) or a Redis command line.
    pub fn execute(&self, query: &str, row_limit: usize) -> Result<QueryOutcome> {
        match self {
            Self::Sql(db) => db.execute(query, row_limit),
            Self::Redis(db) => db.execute(query),
        }
    }

    /// Runs SQL statements in one transaction (all or nothing); returns rows affected.
    pub fn execute_batch(&self, statements: &[String]) -> Result<u64> {
        match self {
            Self::Sql(db) => db.execute_batch(statements),
            Self::Redis(_) => anyhow::bail!("Redis has no transactions for table edits"),
        }
    }

    /// Primary-key columns of a table, in key order (SQL only).
    pub fn primary_key(&self, table: &edit::TableRef) -> Result<Vec<String>> {
        match self {
            Self::Sql(db) => db.primary_key(table),
            Self::Redis(_) => anyhow::bail!("Redis has no tables"),
        }
    }

    /// Schemas, tables and columns (SQL) or keys matching `*` (Redis).
    pub fn schema(&self) -> Result<Vec<SchemaNode>> {
        match self {
            Self::Sql(db) => db.schema(),
            Self::Redis(db) => db.keys("*", DEFAULT_ROW_LIMIT),
        }
    }
}

/// Shared runtime for the async drivers.
fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("db")
            .enable_all()
            .build()
            .expect("failed to start the database runtime")
    })
}
