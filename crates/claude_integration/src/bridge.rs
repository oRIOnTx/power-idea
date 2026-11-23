//! The hook / status-line bridge: `power-idea claude-hook` and `power-idea claude-statusline`
//! read Claude Code's JSON on stdin and forward it to the running IDE over a Unix socket.
//! They never fail the session: no output for hooks, exit 0, short timeouts, IDE closed = no-op.

use std::io::{Read, Write as _};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::launcher::{SOCKET_ENV, TAG_ENV};

/// How long the helpers wait on the IDE before giving up.
const SEND_TIMEOUT: Duration = Duration::from_millis(300);
/// Larger payloads (huge tool inputs) are cut; the gauges need only a few fields.
const MAX_PAYLOAD: usize = 1 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Hook,
    Status,
}

/// One message on the socket: a JSON line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub kind: Kind,
    /// Which IDE window started the session (the project root).
    pub tag: String,
    pub payload: Value,
}

impl Message {
    pub fn encode(&self) -> String {
        let mut line = serde_json::to_string(self).unwrap_or_default();
        line.push('\n');
        line
    }

    pub fn decode(line: &str) -> Option<Self> {
        serde_json::from_str(line.trim()).ok()
    }
}

/// Sends `message` to the socket; any failure (IDE closed, slow) is ignored.
pub fn send(socket: &Path, message: &Message) {
    let Ok(mut stream) = UnixStream::connect(socket) else {
        return;
    };
    let _ = stream.set_write_timeout(Some(SEND_TIMEOUT));
    let _ = stream.write_all(message.encode().as_bytes());
}

fn read_stdin() -> Value {
    let mut text = String::new();
    let _ = std::io::stdin()
        .take(MAX_PAYLOAD as u64)
        .read_to_string(&mut text);
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

fn forward(kind: Kind, payload: &Value) {
    let Some(socket) = std::env::var_os(SOCKET_ENV) else {
        return;
    };
    let tag = std::env::var(TAG_ENV).unwrap_or_default();
    send(
        Path::new(&socket),
        &Message {
            kind,
            tag,
            payload: payload.clone(),
        },
    );
}

/// `power-idea claude-hook`: forward the hook JSON, print nothing, exit 0.
pub fn run_hook() -> i32 {
    let payload = read_stdin();
    if !payload.is_null() {
        forward(Kind::Hook, &payload);
    }
    0
}

/// `power-idea claude-statusline [--user-command-file <path>]`: forward the status JSON, then
/// print the user's own status line (run with the same JSON) or a compact default.
pub fn run_statusline(args: &[String]) -> i32 {
    let payload = read_stdin();
    if !payload.is_null() {
        forward(Kind::Status, &payload);
    }
    let user_command = args
        .iter()
        .position(|a| a == "--user-command-file")
        .and_then(|ix| args.get(ix + 1))
        .and_then(|file| std::fs::read_to_string(file).ok())
        .filter(|c| !c.trim().is_empty());
    let line = match user_command {
        Some(command) => run_user_statusline(&command, &payload).unwrap_or_default(),
        None => default_line(&payload),
    };
    // Claude Code shows whatever the status line command writes to stdout.
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(line.as_bytes());
    let _ = stdout.flush();
    0
}

fn run_user_statusline(command: &str, payload: &Value) -> Option<String> {
    let mut child = Command::new("/bin/sh")
        .args(["-c", command])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(payload.to_string().as_bytes());
    }
    let output = child.wait_with_output().ok()?;
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// `Opus · ctx 42% · $0.37` from the status JSON.
pub fn default_line(payload: &Value) -> String {
    let mut parts = Vec::new();
    if let Some(model) = payload.pointer("/model/display_name").and_then(Value::as_str) {
        parts.push(model.to_string());
    }
    if let Some(percent) = payload
        .pointer("/context_window/used_percentage")
        .and_then(Value::as_f64)
    {
        parts.push(format!("ctx {percent:.0}%"));
    }
    if let Some(cost) = payload.pointer("/cost/total_cost_usd").and_then(Value::as_f64) {
        parts.push(format!("${cost:.2}"));
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::BufRead as _;
    use std::os::unix::net::UnixListener;

    #[test]
    fn round_trips_messages() {
        let message = Message {
            kind: Kind::Hook,
            tag: "/p".into(),
            payload: json!({ "hook_event_name": "Stop" }),
        };
        let line = message.encode();
        assert!(line.ends_with('\n') && !line.trim_end().contains('\n'));
        assert_eq!(Message::decode(&line), Some(message));
        assert_eq!(Message::decode("not json"), None);
    }

    #[test]
    fn sends_over_a_socket_and_ignores_a_missing_ide() {
        let dir = std::env::temp_dir().join(format!("pi-bridge-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("s.sock");
        send(
            &socket,
            &Message {
                kind: Kind::Status,
                tag: String::new(),
                payload: Value::Null,
            },
        );

        let listener = UnixListener::bind(&socket).unwrap();
        let message = Message {
            kind: Kind::Status,
            tag: "/p".into(),
            payload: json!({ "context_window": { "used_percentage": 12 } }),
        };
        send(&socket, &message);
        let (stream, _) = listener.accept().unwrap();
        let mut line = String::new();
        std::io::BufReader::new(stream).read_line(&mut line).unwrap();
        assert_eq!(Message::decode(&line), Some(message));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn formats_the_default_line() {
        let payload = json!({
            "model": { "display_name": "Opus" },
            "context_window": { "used_percentage": 41.6 },
            "cost": { "total_cost_usd": 0.372 },
        });
        assert_eq!(default_line(&payload), "Opus · ctx 42% · $0.37");
        assert_eq!(default_line(&Value::Null), "");
    }

    #[test]
    fn runs_the_users_status_line_with_the_json() {
        let out = run_user_statusline("cat", &json!({ "a": 1 })).unwrap();
        assert_eq!(out, r#"{"a":1}"#);
    }
}
