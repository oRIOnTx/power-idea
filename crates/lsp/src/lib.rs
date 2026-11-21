//! Minimal Language Server Protocol client over stdio.
//!
//! One reader thread per server parses `Content-Length` framed JSON-RPC,
//! routes responses to waiting callers and hands notifications to a callback.
//! Requests block the calling thread, so call them from background tasks.

mod uri;

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail};
use lsp_types::notification::Notification;
use lsp_types::request::Request;
use serde_json::{Value, json};

pub use lsp_types;
pub use uri::{path_to_uri, uri_to_path};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
/// Lines of server stderr kept for error reports.
const STDERR_TAIL: usize = 200;
/// Pseudo-notification sent to the handler when the server process goes away.
pub const EXIT_NOTIFICATION: &str = "$/power-idea/exit";

/// Receives server notifications (method, params) on the reader thread.
pub type NotificationHandler = Arc<dyn Fn(&str, Value) + Send + Sync>;

type Pending = Mutex<HashMap<i64, mpsc::Sender<Result<Value>>>>;

pub struct LanguageServer {
    name: String,
    writer: Mutex<Box<dyn Write + Send>>,
    pending: Arc<Pending>,
    next_id: AtomicI64,
    capabilities: Mutex<Value>,
    child: Mutex<Option<Child>>,
    stderr: Arc<Mutex<VecDeque<String>>>,
}

impl LanguageServer {
    /// Spawns `command` in `root` (with extra `env`) and completes the
    /// `initialize` handshake.
    pub fn start(
        name: &str,
        command: &str,
        args: &[String],
        env: &[(String, String)],
        root: &Path,
        on_notification: NotificationHandler,
    ) -> Result<Arc<Self>> {
        let mut child = Command::new(command)
            .args(args)
            .envs(env.iter().map(|(k, v)| (k, v)))
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("starting {command}"))?;
        let stdin = child.stdin.take().context("no stdin")?;
        let stdout = child.stdout.take().context("no stdout")?;
        let stderr = child.stderr.take().context("no stderr")?;
        let server = Self::from_streams(name, stdout, stdin, on_notification);
        server.capture_stderr(stderr);
        *server.child.lock().unwrap() = Some(child);
        if let Err(err) = server.initialize(root) {
            // Give the stderr reader a moment to collect the crash report.
            thread::sleep(Duration::from_millis(300));
            return Err(match server.failure_summary() {
                Some(reason) => anyhow!("{command} exited during startup: {reason}"),
                None => err.context(format!("{command} failed to start")),
            });
        }
        Ok(server)
    }

    /// Connects to a server over arbitrary streams (used by tests).
    pub fn from_streams(
        name: &str,
        reader: impl Read + Send + 'static,
        writer: impl Write + Send + 'static,
        on_notification: NotificationHandler,
    ) -> Arc<Self> {
        let server = Arc::new(Self {
            name: name.to_string(),
            writer: Mutex::new(Box::new(writer)),
            pending: Arc::new(Mutex::new(HashMap::new())),
            next_id: AtomicI64::new(1),
            capabilities: Mutex::new(Value::Null),
            child: Mutex::new(None),
            stderr: Arc::new(Mutex::new(VecDeque::new())),
        });
        let weak = Arc::downgrade(&server);
        let pending = server.pending.clone();
        let _ = thread::Builder::new().name(format!("lsp-{name}")).spawn(move || {
            let mut reader = BufReader::new(reader);
            while let Ok(Some(message)) = read_message(&mut reader) {
                let Some(server) = weak.upgrade() else { break };
                server.dispatch(&message, &on_notification);
            }
            // The server went away: fail everything still waiting.
            for (_, waiter) in pending.lock().unwrap().drain() {
                let _ = waiter.send(Err(anyhow!("language server exited")));
            }
            on_notification(EXIT_NOTIFICATION, Value::Null);
        });
        server
    }

    fn capture_stderr(&self, stderr: impl Read + Send + 'static) {
        let tail = self.stderr.clone();
        let _ = thread::Builder::new()
            .name(format!("lsp-{}-stderr", self.name))
            .spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    let mut tail = tail.lock().unwrap();
                    if tail.len() == STDERR_TAIL {
                        tail.pop_front();
                    }
                    tail.push_back(line);
                }
            });
    }

    /// Recent stderr output of the server process.
    pub fn stderr_tail(&self) -> Vec<String> {
        self.stderr.lock().unwrap().iter().cloned().collect()
    }

    /// The most telling stderr line after a crash (an error message if any).
    pub fn failure_summary(&self) -> Option<String> {
        summarize_failure(&self.stderr_tail())
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether the server advertises `capability` (e.g. "definitionProvider").
    pub fn supports(&self, capability: &str) -> bool {
        let caps = self.capabilities.lock().unwrap();
        caps.get(capability)
            .is_some_and(|v| !v.is_null() && v != &Value::Bool(false))
    }

    /// Characters that should open the completion menu (e.g. ".").
    pub fn completion_triggers(&self) -> Vec<String> {
        let caps = self.capabilities.lock().unwrap();
        caps.pointer("/completionProvider/triggerCharacters")
            .and_then(Value::as_array)
            .map_or_default(|chars| {
                chars
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
    }

    fn initialize(&self, root: &Path) -> Result<()> {
        let root_uri = path_to_uri(root)?;
        let name = root
            .file_name()
            .map_or_default(|n| n.to_string_lossy().into_owned());
        let params = json!({
            "processId": std::process::id(),
            "clientInfo": { "name": "Power IDEA", "version": env!("CARGO_PKG_VERSION") },
            "rootUri": root_uri.as_str(),
            "workspaceFolders": [{ "uri": root_uri.as_str(), "name": name }],
            "capabilities": client_capabilities(),
        });
        let result = self.request_raw("initialize", params)?;
        *self.capabilities.lock().unwrap() = result.get("capabilities").cloned().unwrap_or(Value::Null);
        self.notify_raw("initialized", json!({}))
    }

    /// Sends a typed request and waits for its result.
    pub fn request<R: Request>(&self, params: R::Params) -> Result<R::Result> {
        let result = self.request_raw(R::METHOD, serde_json::to_value(params)?)?;
        serde_json::from_value(result).with_context(|| format!("decoding {} response", R::METHOD))
    }

    /// Sends a typed notification.
    pub fn notify<N: Notification>(&self, params: N::Params) -> Result<()> {
        self.notify_raw(N::METHOD, serde_json::to_value(params)?)
    }

    pub fn request_raw(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        self.pending.lock().unwrap().insert(id, tx);
        let message = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        if let Err(err) = self.send(&message) {
            self.pending.lock().unwrap().remove(&id);
            return Err(err);
        }
        match rx.recv_timeout(REQUEST_TIMEOUT) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => {
                self.pending.lock().unwrap().remove(&id);
                let _ = self.notify_raw("$/cancelRequest", json!({ "id": id }));
                bail!("{method} timed out")
            }
            Err(RecvTimeoutError::Disconnected) => bail!("language server exited"),
        }
    }

    pub fn notify_raw(&self, method: &str, params: Value) -> Result<()> {
        self.send(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
    }

    fn send(&self, message: &Value) -> Result<()> {
        let body = serde_json::to_vec(message)?;
        let mut writer = self.writer.lock().unwrap();
        write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
        writer.write_all(&body)?;
        writer.flush().context("writing to language server")
    }

    fn dispatch(&self, message: &Value, on_notification: &NotificationHandler) {
        let id = message.get("id").cloned();
        match (message.get("method").and_then(Value::as_str), id) {
            // Response to one of our requests.
            (None, Some(id)) => {
                let Some(id) = id.as_i64() else { return };
                let Some(waiter) = self.pending.lock().unwrap().remove(&id) else {
                    return;
                };
                let result = match message.get("error") {
                    Some(error) => Err(anyhow!(
                        "{}",
                        error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("request failed")
                    )),
                    None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
                };
                let _ = waiter.send(result);
            }
            // Request from the server: answer with sensible defaults.
            (Some(method), Some(id)) => {
                let params = message.get("params").cloned().unwrap_or(Value::Null);
                let result = server_request_default(method, &params);
                let _ = self.send(&json!({ "jsonrpc": "2.0", "id": id, "result": result }));
            }
            (Some(method), None) => {
                on_notification(method, message.get("params").cloned().unwrap_or(Value::Null));
            }
            (None, None) => {}
        }
    }
}

impl Drop for LanguageServer {
    fn drop(&mut self) {
        let _ = self.notify_raw("exit", Value::Null);
        if let Some(mut child) = self.child.get_mut().ok().and_then(Option::take) {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Picks the line that explains a crash: the most specific error-looking
/// line, otherwise the last non-empty line.
pub fn summarize_failure(lines: &[String]) -> Option<String> {
    // Most specific first: "Could not find pg" beats "failed to load command".
    const MARKERS: &[&str] = &["Could not", "could not", "not found", "Error", "error:", "failed"];
    let is_frame = |l: &str| l.starts_with("from ");
    // A continuation line must not look like a new message or a stack frame.
    let is_continuation =
        |l: &str| !l.is_empty() && !is_frame(l) && !l.starts_with('/') && !l.contains(".rb:");
    let lines: Vec<&str> = lines.iter().map(|l| l.trim()).collect();
    let start = MARKERS
        .iter()
        .find_map(|marker| lines.iter().position(|l| !is_frame(l) && l.contains(marker)))
        .or_else(|| lines.iter().rposition(|l| !l.is_empty()))?;
    // Messages are often wrapped over a few lines; keep the continuation.
    let message = lines[start..]
        .iter()
        .take(3)
        .enumerate()
        .take_while(|(i, l)| *i == 0 || is_continuation(l))
        .map(|(_, l)| *l)
        .collect::<Vec<_>>()
        .join(" ");
    Some(message.chars().take(400).collect())
}

/// Replies to server-initiated requests the IDE does not handle itself.
fn server_request_default(method: &str, params: &Value) -> Value {
    match method {
        // One `null` per requested section: "use your defaults".
        "workspace/configuration" => {
            let items = params.get("items").and_then(Value::as_array).map_or(0, Vec::len);
            Value::Array(vec![Value::Null; items])
        }
        "workspace/applyEdit" => json!({ "applied": false }),
        _ => Value::Null,
    }
}

fn client_capabilities() -> Value {
    json!({
        "textDocument": {
            "synchronization": { "didSave": true, "dynamicRegistration": false },
            "completion": {
                "completionItem": { "snippetSupport": false, "documentationFormat": ["markdown", "plaintext"] },
                "contextSupport": true,
            },
            "hover": { "contentFormat": ["markdown", "plaintext"] },
            "definition": { "linkSupport": true },
            "declaration": { "linkSupport": true },
            "implementation": { "linkSupport": true },
            "references": {},
            "documentSymbol": { "hierarchicalDocumentSymbolSupport": true },
            "formatting": {},
            "rename": { "prepareSupport": false },
            "publishDiagnostics": { "relatedInformation": false },
        },
        "workspace": {
            "symbol": {},
            "workspaceFolders": true,
            "configuration": true,
            "applyEdit": false,
        },
        "window": { "workDoneProgress": true },
    })
}

/// Reads one `Content-Length` framed message; `Ok(None)` at end of stream.
fn read_message(reader: &mut impl BufRead) -> Result<Option<Value>> {
    let mut length = None;
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(value) = line.strip_prefix("Content-Length:") {
            length = Some(value.trim().parse::<usize>()?);
        }
    }
    let length = length.context("missing Content-Length")?;
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok(Some(serde_json::from_slice(&body)?))
}

#[cfg(test)]
mod tests {
    use std::io::{BufReader, Write};
    use std::os::unix::net::UnixStream;

    use lsp_types::request::GotoDefinition;
    use lsp_types::{GotoDefinitionParams, Position, TextDocumentIdentifier, TextDocumentPositionParams};

    use super::*;

    fn frame(value: &Value) -> Vec<u8> {
        let body = value.to_string();
        format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
    }

    #[test]
    fn summarizes_crash_output() {
        let lines: Vec<String> = [
            "Ruby LSP> Running bundle install",
            "bundler: failed to load command: ruby-lsp",
            "/x/definition.rb:523: Could not find pg-1.5.9 in locally installed gems (Bundler::GemNotFound)",
            "\tfrom /x/runtime.rb:18",
        ]
        .map(String::from)
        .to_vec();
        assert!(
            summarize_failure(&lines)
                .unwrap()
                .contains("Could not find pg-1.5.9")
        );
        assert_eq!(
            summarize_failure(&["", "last words"].map(String::from)).unwrap(),
            "last words"
        );
        assert!(summarize_failure(&[]).is_none());
    }

    #[test]
    fn reads_framed_messages() {
        let mut bytes = frame(&json!({ "a": 1 }));
        bytes.extend(frame(&json!({ "b": 2 })));
        let mut reader = BufReader::new(bytes.as_slice());
        assert_eq!(read_message(&mut reader).unwrap(), Some(json!({ "a": 1 })));
        assert_eq!(read_message(&mut reader).unwrap(), Some(json!({ "b": 2 })));
        assert_eq!(read_message(&mut reader).unwrap(), None);
    }

    /// Fake server: answers `textDocument/definition`, asks the client for
    /// configuration first and then publishes a notification.
    fn fake_server(stream: UnixStream) {
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut writer = stream;
        while let Ok(Some(message)) = read_message(&mut reader) {
            if message["method"] == "textDocument/definition" {
                let config = json!({ "jsonrpc": "2.0", "id": "cfg", "method": "workspace/configuration",
                    "params": { "items": [{}, {}] } });
                writer.write_all(&frame(&config)).unwrap();
                let response = json!({ "jsonrpc": "2.0", "id": message["id"], "result": {
                    "uri": "file:///tmp/a.rb",
                    "range": { "start": { "line": 3, "character": 2 }, "end": { "line": 3, "character": 5 } }
                }});
                writer.write_all(&frame(&response)).unwrap();
                let note = json!({ "jsonrpc": "2.0", "method": "window/logMessage",
                    "params": { "type": 3, "message": "hi" } });
                writer.write_all(&frame(&note)).unwrap();
            } else if message["id"] == "cfg" {
                assert_eq!(message["result"], json!([null, null]));
            }
        }
    }

    #[test]
    fn request_response_and_notifications() {
        let (client_end, server_end) = UnixStream::pair().unwrap();
        thread::spawn(move || fake_server(server_end));

        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let handler: NotificationHandler = Arc::new(move |method, _| {
            let _ = tx.lock().unwrap().send(method.to_string());
        });
        let server =
            LanguageServer::from_streams("fake", client_end.try_clone().unwrap(), client_end, handler);

        let params = GotoDefinitionParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: "file:///tmp/b.rb".parse().unwrap(),
                },
                position: Position::new(0, 0),
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let response = server.request::<GotoDefinition>(params).unwrap();
        let Some(lsp_types::GotoDefinitionResponse::Scalar(location)) = response else {
            panic!("unexpected response: {response:?}");
        };
        assert_eq!(location.range.start, Position::new(3, 2));
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            "window/logMessage"
        );
    }
}
