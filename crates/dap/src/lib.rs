//! Debug Adapter Protocol client, the debugger counterpart of LSP: one
//! protocol for any language's debugger (rdbg for Ruby, lldb-dap for Rust and
//! C/C++, debugpy for Python, dlv for Go). Adapters talk over stdio or a local
//! TCP port. Requests block the calling thread, so call them from background
//! tasks; events arrive on a reader thread.

mod adapters;
pub mod model;

use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow, bail};
use serde_json::{Value, json};

pub use adapters::{AdapterKind, DebugTarget, LaunchPlan, plan};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// How the IDE reaches the adapter process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transport {
    Stdio,
    /// The adapter listens on `127.0.0.1:port` once started.
    Tcp {
        port: u16,
    },
}

/// The adapter process to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterCommand {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
    pub transport: Transport,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DapEvent {
    /// A protocol event (`stopped`, `output`, `terminated`…).
    Event { event: String, body: Value },
    /// Text the adapter or debuggee process printed outside the protocol.
    ProcessOutput(String),
    /// The connection closed.
    Closed,
}

pub type EventHandler = Arc<dyn Fn(DapEvent) + Send + Sync>;

type Pending = Mutex<HashMap<i64, mpsc::Sender<Result<Value>>>>;

pub struct DebugAdapter {
    writer: Mutex<Box<dyn Write + Send>>,
    pending: Arc<Pending>,
    seq: AtomicI64,
    child: Mutex<Option<Child>>,
}

impl std::fmt::Debug for DebugAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DebugAdapter").finish_non_exhaustive()
    }
}

/// A free local TCP port for an adapter to listen on.
pub fn free_port() -> Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").context("finding a free port")?;
    Ok(listener.local_addr()?.port())
}

fn forward_lines(stream: impl Read + Send + 'static, on_event: EventHandler) {
    let _ = thread::Builder::new().name("dap-output".into()).spawn(move || {
        for line in BufReader::new(stream).lines().map_while(Result::ok) {
            on_event(DapEvent::ProcessOutput(line));
        }
    });
}

impl DebugAdapter {
    /// Starts the adapter and connects to it.
    pub fn start(command: &AdapterCommand, on_event: EventHandler) -> Result<Arc<Self>> {
        let mut child = Command::new(&command.program)
            .args(&command.args)
            .envs(&command.env)
            .current_dir(&command.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("starting {}", command.program))?;
        let stderr = child.stderr.take().context("no stderr")?;
        forward_lines(stderr, on_event.clone());
        let adapter = match command.transport {
            Transport::Stdio => {
                let stdin = child.stdin.take().context("no stdin")?;
                let stdout = child.stdout.take().context("no stdout")?;
                Self::from_streams(stdout, stdin, on_event)
            }
            Transport::Tcp { port } => {
                // The debuggee's own output comes on the process streams.
                let stdout = child.stdout.take().context("no stdout")?;
                forward_lines(stdout, on_event.clone());
                let stream = connect(port, &mut child)?;
                let reader = stream.try_clone()?;
                Self::from_streams(reader, stream, on_event)
            }
        };
        *adapter.child.lock().unwrap_or_else(PoisonError::into_inner) = Some(child);
        Ok(adapter)
    }

    /// Talks to an adapter over arbitrary streams (used by tests).
    pub fn from_streams(
        reader: impl Read + Send + 'static,
        writer: impl Write + Send + 'static,
        on_event: EventHandler,
    ) -> Arc<Self> {
        let adapter = Arc::new(Self {
            writer: Mutex::new(Box::new(writer)),
            pending: Arc::new(Mutex::new(HashMap::new())),
            seq: AtomicI64::new(1),
            child: Mutex::new(None),
        });
        let weak = Arc::downgrade(&adapter);
        let pending = adapter.pending.clone();
        let _ = thread::Builder::new().name("dap-reader".into()).spawn(move || {
            let mut reader = BufReader::new(reader);
            while let Ok(Some(message)) = read_message(&mut reader) {
                let Some(adapter) = weak.upgrade() else { break };
                adapter.dispatch(message, &on_event);
            }
            for (_, waiter) in pending.lock().unwrap_or_else(PoisonError::into_inner).drain() {
                let _ = waiter.send(Err(anyhow!("debug adapter exited")));
            }
            on_event(DapEvent::Closed);
        });
        adapter
    }

    /// Sends a request and waits for the response body.
    pub fn request(&self, command: &str, arguments: Value) -> Result<Value> {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(seq, tx);
        let message = json!({ "seq": seq, "type": "request", "command": command, "arguments": arguments });
        if let Err(err) = self.send(&message) {
            self.pending
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&seq);
            return Err(err);
        }
        match rx.recv_timeout(REQUEST_TIMEOUT) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => {
                self.pending
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(&seq);
                bail!("{command} timed out")
            }
            Err(RecvTimeoutError::Disconnected) => bail!("debug adapter exited"),
        }
    }

    fn send(&self, message: &Value) -> Result<()> {
        let body = serde_json::to_vec(message)?;
        let mut writer = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
        writer.write_all(&body)?;
        writer.flush().context("writing to the debug adapter")
    }

    fn dispatch(&self, message: Value, on_event: &EventHandler) {
        match message.get("type").and_then(Value::as_str) {
            Some("response") => {
                let Some(seq) = message.get("request_seq").and_then(Value::as_i64) else {
                    return;
                };
                let Some(waiter) = self
                    .pending
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(&seq)
                else {
                    return;
                };
                let result = if message.get("success").and_then(Value::as_bool) == Some(true) {
                    Ok(message.get("body").cloned().unwrap_or(Value::Null))
                } else {
                    let error = message
                        .pointer("/body/error/format")
                        .or_else(|| message.get("message"))
                        .and_then(Value::as_str)
                        .unwrap_or("request failed");
                    Err(anyhow!("{error}"))
                };
                let _ = waiter.send(result);
            }
            Some("event") => on_event(DapEvent::Event {
                event: message
                    .get("event")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                body: message.get("body").cloned().unwrap_or(Value::Null),
            }),
            // Reverse requests (runInTerminal, startDebugging): not supported.
            Some("request") => {
                let reply = json!({
                    "seq": self.seq.fetch_add(1, Ordering::Relaxed),
                    "type": "response",
                    "request_seq": message.get("seq"),
                    "command": message.get("command"),
                    "success": false,
                    "message": "not supported by Power IDEA",
                });
                let _ = self.send(&reply);
            }
            _ => {}
        }
    }

    /// Ends the adapter (and with it the debuggee when the adapter launched it).
    pub fn kill(&self) {
        if let Some(mut child) = self.child.lock().unwrap_or_else(PoisonError::into_inner).take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for DebugAdapter {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Connects to a TCP adapter, waiting for it to start listening.
fn connect(port: u16, child: &mut Child) -> Result<TcpStream> {
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    loop {
        if let Ok(stream) = TcpStream::connect(("127.0.0.1", port)) {
            return Ok(stream);
        }
        if let Ok(Some(status)) = child.try_wait() {
            bail!("the debugger exited before accepting a connection ({status})");
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            bail!("the debugger did not open port {port}");
        }
        thread::sleep(Duration::from_millis(100));
    }
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
            if length.is_some() {
                break;
            }
            continue;
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
    use std::sync::{Arc, Mutex, PoisonError};

    use serde_json::{Value, json};

    use super::{DapEvent, DebugAdapter, read_message};

    fn frame(value: &Value) -> Vec<u8> {
        let body = value.to_string();
        format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
    }

    #[test]
    fn pairs_responses_events_and_reverse_requests() {
        let (ours, theirs) = UnixStream::pair().unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let adapter = DebugAdapter::from_streams(
            ours.try_clone().unwrap(),
            ours,
            Arc::new(move |event| sink.lock().unwrap_or_else(PoisonError::into_inner).push(event)),
        );
        let fake = std::thread::spawn(move || {
            let mut reader = BufReader::new(theirs.try_clone().unwrap());
            let mut writer = theirs;
            let request = read_message(&mut reader).unwrap().unwrap();
            assert_eq!(request["command"], "threads");
            // An event and a reverse request arrive before the response.
            writer
                .write_all(&frame(
                    &json!({ "type": "event", "event": "output", "body": { "output": "hi" } }),
                ))
                .unwrap();
            writer
                .write_all(&frame(
                    &json!({ "seq": 9, "type": "request", "command": "runInTerminal" }),
                ))
                .unwrap();
            let refused = read_message(&mut reader).unwrap().unwrap();
            assert_eq!(
                (refused["request_seq"].as_i64(), refused["success"].as_bool()),
                (Some(9), Some(false))
            );
            writer
                .write_all(&frame(&json!({
                    "type": "response", "request_seq": request["seq"], "success": true,
                    "body": { "threads": [{ "id": 1, "name": "main" }] }
                })))
                .unwrap();
            let failing = read_message(&mut reader).unwrap().unwrap();
            writer
                .write_all(&frame(&json!({
                    "type": "response", "request_seq": failing["seq"], "success": false, "message": "nope"
                })))
                .unwrap();
        });
        let threads = adapter.request("threads", Value::Null).unwrap();
        assert_eq!(threads["threads"][0]["name"], "main");
        assert_eq!(
            adapter.request("pause", json!({})).unwrap_err().to_string(),
            "nope"
        );
        fake.join().unwrap();
        assert!(
            events
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .any(|e| matches!(
                    e,
                    DapEvent::Event { event, body } if event == "output" && body["output"] == "hi"
                ))
        );
    }
}
