//! SQL databases through `sqlx`. Queries run as raw SQL (simple query
//! protocol), so several statements can be executed at once and values come
//! back as text, which is exactly what a results grid needs.

use std::collections::BTreeMap;
use std::pin::Pin;
use std::time::Instant;

use anyhow::{Context as _, Result};
use futures_core::Stream;
use sqlx::mysql::{MySqlConnectOptions, MySqlPoolOptions, MySqlSslMode};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{
    AssertSqlSafe, Column as _, Either, Executor, MySqlPool, PgPool, Row, SqlitePool, TypeInfo as _,
    ValueRef as _,
};

use super::edit::{TableRef, qualified, quote_literal};
use super::{
    CONNECT_TIMEOUT, ConnectionConfig, DbKind, NodeKind, QueryOutcome, ResultSet, SchemaNode, runtime,
};

pub enum SqlDatabase {
    Postgres(PgPool),
    MySql(MySqlPool),
    Sqlite(SqlitePool),
}

/// Awaits the next item of a stream (avoids pulling in a futures utility crate).
async fn next<S: Stream + ?Sized>(stream: &mut Pin<Box<S>>) -> Option<S::Item> {
    std::future::poll_fn(|cx| stream.as_mut().poll_next(cx)).await
}

/// Collects a `fetch_many` stream into result sets, stopping after `limit` rows per set.
macro_rules! collect_results {
    ($pool:expr, $sql:expr, $limit:expr, $format:path) => {{
        let mut stream = $pool.fetch_many(sqlx::raw_sql(AssertSqlSafe($sql.to_string())));
        let mut outcome = QueryOutcome::default();
        let mut current: Option<ResultSet> = None;
        while let Some(item) = next(&mut stream).await {
            match item? {
                Either::Left(done) => {
                    outcome.rows_affected += done.rows_affected();
                    outcome.sets.extend(current.take());
                }
                Either::Right(row) => {
                    let set = current.get_or_insert_with(|| ResultSet {
                        columns: row.columns().iter().map(|c| c.name().to_string()).collect(),
                        ..Default::default()
                    });
                    if set.rows.len() >= $limit {
                        set.truncated = true;
                        break;
                    }
                    set.rows.push((0..row.len()).map(|i| $format(&row, i)).collect());
                }
            }
        }
        outcome.sets.extend(current.take());
        anyhow::Ok(outcome)
    }};
}

impl SqlDatabase {
    pub fn connect(config: &ConnectionConfig, password: Option<&str>) -> Result<Self> {
        let db = runtime().block_on(async {
            anyhow::Ok(match config.kind {
                DbKind::Postgres => {
                    let mut options = PgConnectOptions::new()
                        .host(&config.host)
                        .port(config.port)
                        .username(&config.user)
                        .ssl_mode(if config.tls {
                            PgSslMode::Require
                        } else {
                            PgSslMode::Prefer
                        });
                    if let Some(password) = password {
                        options = options.password(password);
                    }
                    if !config.database.is_empty() {
                        options = options.database(&config.database);
                    }
                    Self::Postgres(
                        PgPoolOptions::new()
                            .max_connections(2)
                            .acquire_timeout(CONNECT_TIMEOUT)
                            .connect_with(options)
                            .await?,
                    )
                }
                DbKind::MySql => {
                    let mut options = MySqlConnectOptions::new()
                        .host(&config.host)
                        .port(config.port)
                        .username(&config.user)
                        .ssl_mode(if config.tls {
                            MySqlSslMode::Required
                        } else {
                            MySqlSslMode::Preferred
                        });
                    if let Some(password) = password {
                        options = options.password(password);
                    }
                    if !config.database.is_empty() {
                        options = options.database(&config.database);
                    }
                    Self::MySql(
                        MySqlPoolOptions::new()
                            .max_connections(2)
                            .acquire_timeout(CONNECT_TIMEOUT)
                            .connect_with(options)
                            .await?,
                    )
                }
                DbKind::Sqlite => {
                    let options = SqliteConnectOptions::new()
                        .filename(&config.file)
                        .create_if_missing(false);
                    Self::Sqlite(
                        SqlitePoolOptions::new()
                            .max_connections(1)
                            .connect_with(options)
                            .await?,
                    )
                }
                DbKind::Redis => unreachable!("Redis is not an SQL database"),
            })
        })?;
        Ok(db)
    }

    pub fn execute(&self, query: &str, limit: usize) -> Result<QueryOutcome> {
        let started = Instant::now();
        let mut outcome = runtime().block_on(async {
            match self {
                Self::Postgres(pool) => collect_results!(pool, query, limit, format_pg),
                Self::MySql(pool) => collect_results!(pool, query, limit, format_mysql),
                Self::Sqlite(pool) => collect_results!(pool, query, limit, format_sqlite),
            }
        })?;
        outcome.elapsed = started.elapsed();
        Ok(outcome)
    }

    pub fn primary_key(&self, table: &TableRef) -> Result<Vec<String>> {
        let lit = |v: &str| quote_literal(DbKind::Postgres, Some(v));
        let query = match self {
            Self::Postgres(_) => format!(
                "SELECT a.attname FROM pg_index i \
                 JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = ANY(i.indkey) \
                 WHERE i.indisprimary AND i.indrelid = ({}::text)::regclass \
                 ORDER BY array_position(i.indkey, a.attnum)",
                lit(&qualified(DbKind::Postgres, table))
            ),
            Self::MySql(_) => format!(
                "SELECT COLUMN_NAME FROM information_schema.KEY_COLUMN_USAGE \
                 WHERE CONSTRAINT_NAME = 'PRIMARY' AND TABLE_NAME = {} AND TABLE_SCHEMA = {} \
                 ORDER BY ORDINAL_POSITION",
                quote_literal(DbKind::MySql, Some(&table.name)),
                table
                    .schema
                    .as_deref()
                    .map_or_else(|| "DATABASE()".into(), |s| quote_literal(DbKind::MySql, Some(s)))
            ),
            Self::Sqlite(_) => format!(
                "SELECT name FROM pragma_table_info({}) WHERE pk > 0 ORDER BY pk",
                lit(&table.name)
            ),
        };
        let outcome = self
            .execute(&query, usize::MAX)
            .context("reading the primary key")?;
        Ok(outcome.sets.into_iter().next().map_or_default(|set| {
            set.rows
                .into_iter()
                .filter_map(|r| r.into_iter().next().flatten())
                .collect()
        }))
    }

    pub fn schema(&self) -> Result<Vec<SchemaNode>> {
        let query = match self {
            Self::Postgres(_) => {
                "SELECT c.table_schema, c.table_name, t.table_type, c.column_name, c.data_type \
                 FROM information_schema.columns c \
                 JOIN information_schema.tables t ON t.table_schema = c.table_schema AND t.table_name = c.table_name \
                 WHERE c.table_schema NOT IN ('pg_catalog', 'information_schema') \
                 ORDER BY c.table_schema, c.table_name, c.ordinal_position"
            }
            Self::MySql(_) => {
                "SELECT c.TABLE_SCHEMA, c.TABLE_NAME, t.TABLE_TYPE, c.COLUMN_NAME, c.COLUMN_TYPE \
                 FROM information_schema.COLUMNS c \
                 JOIN information_schema.TABLES t ON t.TABLE_SCHEMA = c.TABLE_SCHEMA AND t.TABLE_NAME = c.TABLE_NAME \
                 WHERE c.TABLE_SCHEMA = DATABASE() \
                 ORDER BY c.TABLE_NAME, c.ORDINAL_POSITION"
            }
            Self::Sqlite(_) => {
                "SELECT 'main', m.name, m.type, p.name, p.type FROM sqlite_master m \
                 JOIN pragma_table_info(m.name) p \
                 WHERE m.type IN ('table', 'view') AND m.name NOT LIKE 'sqlite_%' \
                 ORDER BY m.name, p.cid"
            }
        };
        let outcome = self.execute(query, usize::MAX).context("reading the schema")?;
        let rows = outcome.sets.into_iter().next().map_or_default(|s| s.rows);
        Ok(build_schema_tree(rows))
    }
}

/// Groups (schema, table, table_type, column, data_type) rows into a tree.
fn build_schema_tree(rows: Vec<Vec<Option<String>>>) -> Vec<SchemaNode> {
    let mut schemas: BTreeMap<String, BTreeMap<String, SchemaNode>> = BTreeMap::new();
    for row in rows {
        let text = |i: usize| row.get(i).cloned().flatten().unwrap_or_default();
        let (schema, table, table_type) = (text(0), text(1), text(2).to_ascii_lowercase());
        let kind = if table_type.contains("view") {
            NodeKind::View
        } else {
            NodeKind::Table
        };
        let node = schemas
            .entry(schema)
            .or_default()
            .entry(table.clone())
            .or_insert_with(|| SchemaNode {
                name: table,
                kind,
                children: Vec::new(),
            });
        node.children.push(SchemaNode {
            name: text(3),
            kind: NodeKind::Column { data_type: text(4) },
            children: Vec::new(),
        });
    }
    schemas
        .into_iter()
        .map(|(name, tables)| SchemaNode {
            name,
            kind: NodeKind::Schema,
            children: tables.into_values().collect(),
        })
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::from("0x");
    for b in bytes.iter().take(64) {
        out.push_str(&format!("{b:02x}"));
    }
    if bytes.len() > 64 {
        out.push('…');
    }
    out
}

fn format_pg(row: &sqlx::postgres::PgRow, i: usize) -> Option<String> {
    let raw = row.try_get_raw(i).ok()?;
    if raw.is_null() {
        return None;
    }
    let is_bool = raw.type_info().name() == "BOOL";
    // Simple query protocol: every value arrives in text format.
    let text = row.try_get_unchecked::<String, _>(i).unwrap_or_else(|_| {
        row.try_get_unchecked::<Vec<u8>, _>(i)
            .map_or_else(|_| "?".into(), |b| hex(&b))
    });
    Some(match (is_bool, text.as_str()) {
        (true, "t") => "true".into(),
        (true, "f") => "false".into(),
        _ => text,
    })
}

fn format_mysql(row: &sqlx::mysql::MySqlRow, i: usize) -> Option<String> {
    let raw = row.try_get_raw(i).ok()?;
    if raw.is_null() {
        return None;
    }
    Some(row.try_get_unchecked::<String, _>(i).unwrap_or_else(|_| {
        row.try_get_unchecked::<Vec<u8>, _>(i)
            .map_or_else(|_| "?".into(), |b| hex(&b))
    }))
}

fn format_sqlite(row: &sqlx::sqlite::SqliteRow, i: usize) -> Option<String> {
    let raw = row.try_get_raw(i).ok()?;
    if raw.is_null() {
        return None;
    }
    // SQLite values are dynamically typed; try the storage classes in turn.
    if let Ok(v) = row.try_get_unchecked::<i64, _>(i)
        && raw.type_info().name() == "INTEGER"
    {
        return Some(v.to_string());
    }
    if let Ok(v) = row.try_get_unchecked::<f64, _>(i)
        && raw.type_info().name() == "REAL"
    {
        return Some(v.to_string());
    }
    if raw.type_info().name() == "BLOB" {
        return row.try_get_unchecked::<Vec<u8>, _>(i).ok().map(|b| hex(&b));
    }
    row.try_get_unchecked::<String, _>(i).ok()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn sqlite_db() -> (SqlDatabase, std::path::PathBuf) {
        // A counter, not the clock: parallel tests must never share a file.
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "power-idea-db-{}-{}.sqlite",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::File::create(&path).unwrap();
        let config = ConnectionConfig {
            kind: DbKind::Sqlite,
            file: path.display().to_string(),
            ..Default::default()
        };
        (SqlDatabase::connect(&config, None).unwrap(), path)
    }

    #[test]
    fn sqlite_queries_schema_and_limits() {
        let (db, path) = sqlite_db();
        let created = db
            .execute(
                "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, score REAL, avatar BLOB); \
                 INSERT INTO users (name, score, avatar) VALUES ('ann', 1.5, x'ff00'), ('bob', NULL, NULL), ('cid', 3, NULL);",
                10,
            )
            .unwrap();
        assert_eq!(created.rows_affected, 3);

        let result = db
            .execute("SELECT id, name, score, avatar FROM users ORDER BY id", 2)
            .unwrap();
        let set = &result.sets[0];
        assert_eq!(set.columns, ["id", "name", "score", "avatar"]);
        assert_eq!(set.rows.len(), 2);
        assert!(set.truncated);
        assert_eq!(
            set.rows[0],
            [
                Some("1".into()),
                Some("ann".into()),
                Some("1.5".into()),
                Some("0xff00".into())
            ]
        );
        assert_eq!(set.rows[1][2], None);

        let schema = db.schema().unwrap();
        let users = &schema[0].children[0];
        assert_eq!((users.name.as_str(), &users.kind), ("users", &NodeKind::Table));
        assert_eq!(users.children.len(), 4);
        assert_eq!(
            users.children[1].kind,
            NodeKind::Column {
                data_type: "TEXT".into()
            }
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn edits_rows_by_primary_key() {
        use crate::edit::{CellEdit, editable_table, update_statement};

        let (db, path) = sqlite_db();
        db.execute(
            "CREATE TABLE memberships (org TEXT, user_id INTEGER, role TEXT, PRIMARY KEY (org, user_id)); \
             INSERT INTO memberships VALUES ('acme', 1, 'member'), ('acme', 2, 'member');",
            10,
        )
        .unwrap();
        let table = editable_table("SELECT * FROM memberships ORDER BY user_id").unwrap();
        assert_eq!(db.primary_key(&table).unwrap(), ["org", "user_id"]);

        let edit = CellEdit {
            column: "role".into(),
            value: Some("o'wner".into()),
            key: vec![
                ("org".into(), Some("acme".into())),
                ("user_id".into(), Some("2".into())),
            ],
        };
        let outcome = db
            .execute(&update_statement(DbKind::Sqlite, &table, &edit), 10)
            .unwrap();
        assert_eq!(outcome.rows_affected, 1);
        let roles = db
            .execute("SELECT role FROM memberships ORDER BY user_id", 10)
            .unwrap();
        assert_eq!(
            roles.sets[0].rows,
            [[Some("member".to_string())], [Some("o'wner".to_string())]]
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn reports_sql_errors() {
        let (db, path) = sqlite_db();
        let err = db.execute("SELEC nonsense", 10).unwrap_err();
        assert!(format!("{err:#}").contains("syntax"), "{err:#}");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn elapsed_is_measured() {
        let (db, path) = sqlite_db();
        let result = db.execute("SELECT 1", 10).unwrap();
        assert!(result.elapsed < Duration::from_secs(5));
        let _ = std::fs::remove_file(path);
    }
}
