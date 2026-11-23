//! Two-way JSON-RPC 2.0 over `Content-Length` framed streams (the LSP framing):
//! the IDE sends requests and notifications and answers the extension's requests.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail};
use serde_json::{Value, json};

/// JSON-RPC error codes used in answers to the extension.
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;

/// Lines of extension stderr kept for crash reports.
const STDERR_TAIL: usize = 100;

/// A message from the extension that is not an answer to one of ours.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    Notification {
        method: String,
        params: Value,
    },
    /// The stream closed: the process exited or crashed.
    Closed,
}

/// Receives extension messages on the reader thread.
pub type Handler = Arc<dyn Fn(Incoming) + Send + Sync>;

type Pending = Mutex<HashMap<i64, mpsc::Sender<Result<Value>>>>;

pub struct Connection {
    writer: Mutex<Box<dyn Write + Send>>,
    pending: Arc<Pending>,
    next_id: AtomicI64,
    stderr: Arc<Mutex<VecDeque<String>>>,
}

impl Connection {
    /// Starts reading `reader` on a thread named `name`.
    pub fn new(
        name: &str,
        reader: impl Read + Send + 'static,
        writer: impl Write + Send + 'static,
        handler: Handler,
    ) -> Arc<Self> {
        let connection = Arc::new(Self {
            writer: Mutex::new(Box::new(writer)),
            pending: Arc::new(Mutex::new(HashMap::new())),
            next_id: AtomicI64::new(1),
            stderr: Arc::new(Mutex::new(VecDeque::new())),
        });
        let pending = connection.pending.clone();
        let _ = thread::Builder::new().name(format!("ext-{name}")).spawn(move || {
            let mut reader = BufReader::new(reader);
            while let Ok(Some(message)) = read_message(&mut reader) {
                dispatch(message, &pending, &handler);
            }
            for (_, waiter) in pending.lock().unwrap().drain() {
                let _ = waiter.send(Err(anyhow!("the extension exited")));
            }
            handler(Incoming::Closed);
        });
        connection
    }

    /// Keeps the last lines of `stderr` for `stderr_tail`, appending each to `log`.
    pub fn capture_stderr(
        &self,
        stderr: impl Read + Send + 'static,
        mut log: Option<impl Write + Send + 'static>,
    ) {
        let tail = self.stderr.clone();
        let _ = thread::Builder::new().name("ext-stderr".into()).spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if let Some(log) = log.as_mut() {
                    let _ = writeln!(log, "{line}");
                }
                let mut tail = tail.lock().unwrap();
                if tail.len() == STDERR_TAIL {
                    tail.pop_front();
                }
                tail.push_back(line);
            }
        });
    }

    pub fn stderr_tail(&self) -> Vec<String> {
        self.stderr.lock().unwrap().iter().cloned().collect()
    }

    /// Sends a request and blocks until the answer (or `timeout`); call off the UI thread.
    pub fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        self.pending.lock().unwrap().insert(id, tx);
        let message = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        if let Err(err) = self.send(&message) {
            self.pending.lock().unwrap().remove(&id);
            return Err(err);
        }
        match rx.recv_timeout(timeout) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => {
                self.pending.lock().unwrap().remove(&id);
                bail!("{method} timed out")
            }
            Err(RecvTimeoutError::Disconnected) => bail!("the extension exited"),
        }
    }

    pub fn notify(&self, method: &str, params: Value) -> Result<()> {
        self.send(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
    }

    /// Answers an extension request: a result or an `(error code, message)`.
    pub fn respond(&self, id: Value, result: Result<Value, (i64, String)>) -> Result<()> {
        let message = match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err((code, message)) => {
                json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
            }
        };
        self.send(&message)
    }

    fn send(&self, message: &Value) -> Result<()> {
        let body = serde_json::to_vec(message)?;
        let mut writer = self.writer.lock().unwrap();
        write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
        writer.write_all(&body)?;
        writer.flush().context("writing to the extension")
    }
}

fn dispatch(message: Value, pending: &Pending, handler: &Handler) {
    let method = message.get("method").and_then(Value::as_str).map(str::to_string);
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    match (method, message.get("id").cloned()) {
        (Some(method), Some(id)) => handler(Incoming::Request { id, method, params }),
        (Some(method), None) => handler(Incoming::Notification { method, params }),
        (None, Some(id)) => {
            let Some(waiter) = id.as_i64().and_then(|id| pending.lock().unwrap().remove(&id)) else {
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
        (None, None) => {}
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
    use std::os::unix::net::UnixStream;

    use super::*;

    fn frame(value: &Value) -> Vec<u8> {
        let body = value.to_string();
        format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
    }

    #[test]
    fn reads_framed_messages() {
        let mut bytes = frame(&json!({ "a": 1 }));
        bytes.extend(b"Content-Type: x\r\n");
        bytes.extend(frame(&json!({ "b": "ü" })));
        let mut reader = BufReader::new(&bytes[..]);
        assert_eq!(read_message(&mut reader).unwrap(), Some(json!({ "a": 1 })));
        assert_eq!(read_message(&mut reader).unwrap(), Some(json!({ "b": "ü" })));
        assert_eq!(read_message(&mut reader).unwrap(), None);
    }

    /// A fake extension: asks the IDE for something, then answers our request with it.
    #[test]
    fn requests_both_ways() {
        let (ours, theirs) = UnixStream::pair().unwrap();
        let incoming = Arc::new(Mutex::new(Vec::new()));
        let sink = incoming.clone();
        let connection = Connection::new(
            "test",
            ours.try_clone().unwrap(),
            ours,
            Arc::new(move |message| sink.lock().unwrap().push(message)),
        );

        let extension = thread::spawn(move || {
            let mut writer = theirs.try_clone().unwrap();
            let mut reader = BufReader::new(theirs);
            writer
                .write_all(&frame(
                    &json!({ "jsonrpc": "2.0", "id": "x1", "method": "workspace/info" }),
                ))
                .unwrap();
            let answer = read_message(&mut reader).unwrap().unwrap();
            let request = read_message(&mut reader).unwrap().unwrap();
            writer
                .write_all(&frame(
                    &json!({ "jsonrpc": "2.0", "id": request["id"], "result": answer["result"] }),
                ))
                .unwrap();
            writer
                .write_all(&frame(
                    &json!({ "jsonrpc": "2.0", "method": "log", "params": [1] }),
                ))
                .unwrap();
        });

        while incoming.lock().unwrap().is_empty() {
            thread::sleep(Duration::from_millis(5));
        }
        let Incoming::Request { id, method, .. } = incoming.lock().unwrap()[0].clone() else {
            panic!("expected a request");
        };
        assert_eq!((id.clone(), method.as_str()), (json!("x1"), "workspace/info"));
        connection.respond(id, Ok(json!({ "root": "/p" }))).unwrap();
        let result = connection.request("command/execute", json!({}), Duration::from_secs(5));
        assert_eq!(result.unwrap(), json!({ "root": "/p" }));

        extension.join().unwrap();
        while incoming.lock().unwrap().last() != Some(&Incoming::Closed) {
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            incoming.lock().unwrap()[1],
            Incoming::Notification {
                method: "log".into(),
                params: json!([1])
            }
        );
        let failed = connection.request("again", json!({}), Duration::from_secs(5));
        assert!(failed.is_err());
    }
}
