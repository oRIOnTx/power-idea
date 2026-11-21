//! Minimal MCP server over the Streamable HTTP transport (JSON responses only).
//!
//! Binds to `127.0.0.1`, requires a bearer token and rejects non-local
//! `Origin` headers (DNS rebinding protection), as the MCP spec recommends.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

pub const SERVER_NAME: &str = "power-idea";
const DEFAULT_PROTOCOL_VERSION: &str = "2025-06-18";
const MAX_BODY: usize = 8 * 1024 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(30);

pub struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    /// JSON Schema of the tool arguments.
    pub input_schema: Value,
}

/// Implemented by the application to expose its tools.
pub trait Handler: Send + Sync + 'static {
    fn tools(&self) -> Vec<Tool>;
    /// Runs a tool; `Err` is reported to the client as a tool error.
    fn call(&self, name: &str, args: Value) -> Result<String, String>;
}

/// Starts the server on `127.0.0.1:port` (0 picks a free port) and returns its address.
pub fn serve(port: u16, token: String, handler: Arc<dyn Handler>) -> io::Result<SocketAddr> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    let addr = listener.local_addr()?;
    let token = Arc::new(token);
    thread::Builder::new().name("mcp-accept".into()).spawn(move || {
        for stream in listener.incoming().flatten() {
            let (token, handler) = (token.clone(), handler.clone());
            let _ = thread::Builder::new()
                .name("mcp-conn".into())
                .spawn(move || handle_connection(stream, &token, handler.as_ref()));
        }
    })?;
    Ok(addr)
}

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

fn read_request(stream: &TcpStream) -> io::Result<Request> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();

    let mut headers = Vec::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            headers.push((key.trim().to_string(), value.trim().to_string()));
        }
    }

    let mut request = Request {
        method,
        path,
        headers,
        body: Vec::new(),
    };
    let len: usize = request
        .header("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    if len > MAX_BODY {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "body too large"));
    }
    request.body.resize(len, 0);
    reader.read_exact(&mut request.body)?;
    Ok(request)
}

fn write_response(mut stream: &TcpStream, status: &str, body: Option<&Value>) -> io::Result<()> {
    let body = body.map_or_default(Value::to_string);
    let content_type = if body.is_empty() {
        ""
    } else {
        "Content-Type: application/json\r\n"
    };
    write!(
        stream,
        "HTTP/1.1 {status}\r\n{content_type}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    stream.flush()
}

fn is_local_origin(origin: &str) -> bool {
    let host = origin.split("://").nth(1).unwrap_or(origin);
    let host = host.split('/').next().unwrap_or(host);
    let host = host.rsplit_once(':').map_or(host, |(h, port)| {
        if port.chars().all(|c| c.is_ascii_digit()) {
            h
        } else {
            host
        }
    });
    matches!(host, "localhost" | "127.0.0.1" | "[::1]")
}

/// Constant-time comparison so the token cannot be probed byte by byte.
fn token_matches(given: &str, expected: &str) -> bool {
    given.len() == expected.len()
        && given
            .bytes()
            .zip(expected.bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

fn handle_connection(stream: TcpStream, token: &str, handler: &dyn Handler) {
    let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
    let Ok(request) = read_request(&stream) else {
        let _ = write_response(&stream, "400 Bad Request", None);
        return;
    };

    if request.header("origin").is_some_and(|o| !is_local_origin(o)) {
        let _ = write_response(&stream, "403 Forbidden", None);
        return;
    }
    let authorized = request
        .header("authorization")
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|given| token_matches(given.trim(), token));
    if !authorized {
        let _ = write_response(&stream, "401 Unauthorized", None);
        return;
    }
    if request.path.split('?').next() != Some("/mcp") {
        let _ = write_response(&stream, "404 Not Found", None);
        return;
    }
    if request.method != "POST" {
        // No server-initiated stream (GET) or sessions (DELETE).
        let _ = write_response(&stream, "405 Method Not Allowed", None);
        return;
    }

    let message: Value = match serde_json::from_slice(&request.body) {
        Ok(message) => message,
        Err(err) => {
            let error = rpc_error(Value::Null, -32700, &format!("parse error: {err}"));
            let _ = write_response(&stream, "400 Bad Request", Some(&error));
            return;
        }
    };
    match handle_message(&message, handler) {
        Some(response) => {
            let _ = write_response(&stream, "200 OK", Some(&response));
        }
        None => {
            let _ = write_response(&stream, "202 Accepted", None);
        }
    }
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// Handles one JSON-RPC message; returns `None` for notifications and responses.
pub fn handle_message(message: &Value, handler: &dyn Handler) -> Option<Value> {
    let id = message.get("id").cloned()?;
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        // A response from the client; nothing to answer.
        return None;
    };
    let params = message.get("params").cloned().unwrap_or(Value::Null);

    let result = match method {
        "initialize" => {
            let version = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(DEFAULT_PROTOCOL_VERSION);
            json!({
                "protocolVersion": version,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION") },
                "instructions": "Tools of the Power IDEA editor the user is working in: \
                    open files, cursor and selection, project search, git and editor navigation.",
            })
        }
        "ping" => json!({}),
        "tools/list" => {
            let tools: Vec<Value> = handler
                .tools()
                .into_iter()
                .map(|t| json!({ "name": t.name, "description": t.description, "inputSchema": t.input_schema }))
                .collect();
            json!({ "tools": tools })
        }
        "tools/call" => {
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return Some(rpc_error(id, -32602, "missing tool name"));
            };
            if !handler.tools().iter().any(|t| t.name == name) {
                return Some(rpc_error(id, -32602, &format!("unknown tool: {name}")));
            }
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            let (text, is_error) = match handler.call(name, args) {
                Ok(text) => (text, false),
                Err(text) => (text, true),
            };
            json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
        }
        _ => return Some(rpc_error(id, -32601, &format!("method not found: {method}"))),
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Echo;

    impl Handler for Echo {
        fn tools(&self) -> Vec<Tool> {
            vec![Tool {
                name: "echo",
                description: "Echo text",
                input_schema: json!({ "type": "object", "properties": { "text": { "type": "string" } } }),
            }]
        }

        fn call(&self, _: &str, args: Value) -> Result<String, String> {
            args["text"]
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| "missing text".into())
        }
    }

    fn rpc(method: &str, params: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })
    }

    #[test]
    fn handles_protocol_messages() {
        let init = handle_message(
            &rpc("initialize", json!({ "protocolVersion": "2025-03-26" })),
            &Echo,
        )
        .unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");

        let notification = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        assert!(handle_message(&notification, &Echo).is_none());

        let list = handle_message(&rpc("tools/list", json!({})), &Echo).unwrap();
        assert_eq!(list["result"]["tools"][0]["name"], "echo");

        let call = rpc(
            "tools/call",
            json!({ "name": "echo", "arguments": { "text": "hi" } }),
        );
        let result = handle_message(&call, &Echo).unwrap();
        assert_eq!(result["result"]["content"][0]["text"], "hi");
        assert_eq!(result["result"]["isError"], false);

        let bad = rpc("tools/call", json!({ "name": "echo", "arguments": {} }));
        assert_eq!(handle_message(&bad, &Echo).unwrap()["result"]["isError"], true);

        let unknown = handle_message(&rpc("nope", json!({})), &Echo).unwrap();
        assert_eq!(unknown["error"]["code"], -32601);
    }

    #[test]
    fn checks_origin_and_token() {
        assert!(is_local_origin("http://localhost:3000"));
        assert!(is_local_origin("http://127.0.0.1"));
        assert!(!is_local_origin("https://evil.example"));
        assert!(!is_local_origin("http://localhost.evil.example"));
        assert!(token_matches("abc", "abc"));
        assert!(!token_matches("abd", "abc"));
        assert!(!token_matches("ab", "abc"));
    }

    fn post(addr: SocketAddr, token: Option<&str>, body: &str) -> String {
        let mut stream = TcpStream::connect(addr).unwrap();
        let auth = token.map_or_default(|t| format!("Authorization: Bearer {t}\r\n"));
        write!(
            stream,
            "POST /mcp HTTP/1.1\r\nHost: x\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }

    #[test]
    fn http_roundtrip_requires_token() {
        let addr = serve(0, "secret".into(), Arc::new(Echo)).unwrap();
        let body = rpc(
            "tools/call",
            json!({ "name": "echo", "arguments": { "text": "over http" } }),
        )
        .to_string();

        assert!(post(addr, None, &body).starts_with("HTTP/1.1 401"));
        assert!(post(addr, Some("wrong!"), &body).starts_with("HTTP/1.1 401"));
        let ok = post(addr, Some("secret"), &body);
        assert!(ok.starts_with("HTTP/1.1 200"), "{ok}");
        assert!(ok.contains("over http"));
    }
}
