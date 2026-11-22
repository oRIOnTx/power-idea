//! Redis: run command lines (like `redis-cli`) and browse keys.

use std::sync::Mutex;
use std::time::Instant;

use anyhow::{Context as _, Result, bail};
use redis::Value;

use super::{CONNECT_TIMEOUT, ConnectionConfig, NodeKind, QueryOutcome, ResultSet, SchemaNode};

pub struct RedisDatabase {
    connection: Mutex<redis::Connection>,
}

fn percent_encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Builds `redis://[user[:password]@]host:port/db` (`rediss://` with TLS).
fn url(config: &ConnectionConfig, password: Option<&str>) -> String {
    let auth = match (config.user.is_empty(), password) {
        (true, None) => String::new(),
        (true, Some(pw)) => format!(":{}@", percent_encode(pw)),
        (false, None) => format!("{}@", percent_encode(&config.user)),
        (false, Some(pw)) => format!("{}:{}@", percent_encode(&config.user), percent_encode(pw)),
    };
    let db = config.database.trim().parse::<u32>().unwrap_or(0);
    let scheme = if config.tls { "rediss" } else { "redis" };
    format!("{scheme}://{auth}{}:{}/{db}", config.host, config.port)
}

/// Splits a command line into arguments, honoring single and double quotes.
pub fn split_command(line: &str) -> Result<Vec<String>> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut in_arg = false;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('"'), '\\') => current.extend(chars.next()),
            (Some(_), c) => current.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                in_arg = true;
            }
            (None, c) if c.is_whitespace() => {
                if in_arg {
                    args.push(std::mem::take(&mut current));
                    in_arg = false;
                }
            }
            (None, c) => {
                current.push(c);
                in_arg = true;
            }
        }
    }
    if quote.is_some() {
        bail!("unterminated quote");
    }
    if in_arg {
        args.push(current);
    }
    Ok(args)
}

fn display(value: &Value) -> String {
    match value {
        Value::Nil => "(nil)".into(),
        Value::Int(i) => i.to_string(),
        Value::BulkString(bytes) => String::from_utf8_lossy(bytes).into_owned(),
        Value::SimpleString(s) => s.clone(),
        Value::Okay => "OK".into(),
        Value::Double(d) => d.to_string(),
        Value::Array(items) | Value::Set(items) => {
            format!("[{}]", items.iter().map(display).collect::<Vec<_>>().join(", "))
        }
        Value::Map(pairs) => format!(
            "{{{}}}",
            pairs
                .iter()
                .map(|(k, v)| format!("{}: {}", display(k), display(v)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        other => format!("{other:?}"),
    }
}

/// Turns a reply into a table: lists become indexed rows, maps key/value rows.
fn to_result_set(value: &Value) -> ResultSet {
    let row = |cells: Vec<String>| cells.into_iter().map(Some).collect::<Vec<_>>();
    match value {
        Value::Array(items) | Value::Set(items) => ResultSet {
            columns: vec!["#".into(), "value".into()],
            rows: items
                .iter()
                .enumerate()
                .map(|(i, v)| row(vec![(i + 1).to_string(), display(v)]))
                .collect(),
            truncated: false,
        },
        Value::Map(pairs) => ResultSet {
            columns: vec!["field".into(), "value".into()],
            rows: pairs
                .iter()
                .map(|(k, v)| row(vec![display(k), display(v)]))
                .collect(),
            truncated: false,
        },
        other => ResultSet {
            columns: vec!["result".into()],
            rows: vec![row(vec![display(other)])],
            truncated: false,
        },
    }
}

impl RedisDatabase {
    pub fn connect(config: &ConnectionConfig, password: Option<&str>) -> Result<Self> {
        let client = redis::Client::open(url(config, password)).context("invalid Redis address")?;
        let mut connection = client.get_connection_with_timeout(CONNECT_TIMEOUT)?;
        redis::cmd("PING").query::<String>(&mut connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    /// Runs each non-empty line as one command; the last reply is returned as a table.
    pub fn execute(&self, input: &str) -> Result<QueryOutcome> {
        let started = Instant::now();
        let mut connection = self.connection.lock().unwrap();
        let mut outcome = QueryOutcome::default();
        for line in input
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
        {
            let args = split_command(line)?;
            let mut command = redis::cmd(&args[0]);
            for arg in &args[1..] {
                command.arg(arg);
            }
            let value: Value = command
                .query(&mut *connection)
                .with_context(|| format!("`{line}` failed"))?;
            outcome.sets = vec![to_result_set(&value)];
        }
        outcome.elapsed = started.elapsed();
        Ok(outcome)
    }

    /// Up to `limit` keys matching `pattern` (SCAN, never KEYS) with their types.
    pub fn keys(&self, pattern: &str, limit: usize) -> Result<Vec<SchemaNode>> {
        let mut connection = self.connection.lock().unwrap();
        let mut keys: Vec<String> = Vec::new();
        let mut cursor: u64 = 0;
        loop {
            let (next, batch): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(pattern)
                .arg("COUNT")
                .arg(200)
                .query(&mut *connection)?;
            keys.extend(batch);
            cursor = next;
            if cursor == 0 || keys.len() >= limit {
                break;
            }
        }
        keys.truncate(limit);
        keys.sort();
        let mut pipe = redis::pipe();
        for key in &keys {
            pipe.cmd("TYPE").arg(key);
        }
        let types: Vec<String> = if keys.is_empty() {
            Vec::new()
        } else {
            pipe.query(&mut *connection)?
        };
        Ok(keys
            .into_iter()
            .zip(types)
            .map(|(name, key_type)| SchemaNode {
                name,
                kind: NodeKind::Key { key_type },
                children: Vec::new(),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_command_lines() {
        assert_eq!(split_command("GET user:1").unwrap(), ["GET", "user:1"]);
        assert_eq!(
            split_command(r#"SET k "hello world""#).unwrap(),
            ["SET", "k", "hello world"]
        );
        assert_eq!(split_command("SET k ''").unwrap(), ["SET", "k", ""]);
        assert_eq!(split_command(r#"SET k "a\"b""#).unwrap(), ["SET", "k", "a\"b"]);
        assert!(split_command("GET 'oops").is_err());
    }

    #[test]
    fn builds_urls_with_escaped_credentials() {
        let config = ConnectionConfig {
            kind: super::super::DbKind::Redis,
            host: "cache".into(),
            port: 6380,
            user: "app".into(),
            database: "2".into(),
            ..Default::default()
        };
        assert_eq!(
            url(&config, Some("p@ss:word")),
            "redis://app:p%40ss%3Aword@cache:6380/2"
        );
        let tls = ConnectionConfig {
            tls: true,
            ..config.clone()
        };
        assert_eq!(url(&tls, None), "rediss://app@cache:6380/2");
        // A rediss:// URL is accepted, i.e. TLS support is compiled in.
        assert!(redis::Client::open(url(&tls, None)).is_ok());
        assert_eq!(
            url(
                &ConnectionConfig {
                    user: String::new(),
                    ..config
                },
                None
            ),
            "redis://cache:6380/2"
        );
    }

    #[test]
    fn formats_replies_as_tables() {
        let list = Value::Array(vec![Value::BulkString(b"a".to_vec()), Value::Int(2)]);
        let set = to_result_set(&list);
        assert_eq!(set.columns, ["#", "value"]);
        assert_eq!(set.rows[1], [Some("2".into()), Some("2".into())]);
        assert_eq!(to_result_set(&Value::Nil).rows, [[Some("(nil)".to_string())]]);
    }
}
