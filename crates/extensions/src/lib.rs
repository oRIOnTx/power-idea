//! Extensions: separate processes that talk JSON-RPC over stdio with the IDE.
//! Discovery and manifests, the protocol, and starting/stopping a process (no UI).

pub mod manifest;
pub mod protocol;
pub mod rpc;

use std::fs::{self, OpenOptions};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use serde_json::Value;

pub use manifest::{CommandSpec, Discovery, Extension, Manifest, discover};
pub use protocol::{HostRequest, Level, PickItem, Position, Range};
pub use rpc::{Handler, Incoming};

const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(10);
/// Commands may wait on the user (pickers, inputs), so they get a generous limit.
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(600);
/// How long an extension gets to exit after `shutdown` before it is killed.
const SHUTDOWN_GRACE: Duration = Duration::from_millis(500);

/// A running extension process for one project.
pub struct Host {
    connection: Arc<rpc::Connection>,
    child: Mutex<Option<Child>>,
}

impl Host {
    /// Starts `extension` for the project at `root` and completes `initialize`.
    /// Blocks up to a few seconds: call off the UI thread. Stderr is appended to `log`.
    pub fn start(
        extension: &Extension,
        root: &Path,
        log: Option<&Path>,
        handler: Handler,
    ) -> Result<Arc<Self>> {
        let program = extension.program();
        let mut child = Command::new(&program)
            .args(&extension.manifest.args)
            .current_dir(&extension.dir)
            .env("POWER_IDEA_PROJECT", root)
            .env("POWER_IDEA_API_VERSION", protocol::API_VERSION.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("starting {}", program.display()))?;
        let stdin = child.stdin.take().context("no stdin")?;
        let stdout = child.stdout.take().context("no stdout")?;
        let stderr = child.stderr.take().context("no stderr")?;
        let connection = rpc::Connection::new(extension.id(), stdout, stdin, handler);
        connection.capture_stderr(stderr, log.and_then(open_log));
        let host = Arc::new(Self {
            connection,
            child: Mutex::new(Some(child)),
        });

        let name = root
            .file_name()
            .map_or_default(|n| n.to_string_lossy().into_owned());
        let params = protocol::initialize_params(
            &root.to_string_lossy(),
            &name,
            extension.id(),
            &extension.dir.to_string_lossy(),
        );
        if let Err(err) = host
            .connection
            .request(protocol::INITIALIZE, params, INITIALIZE_TIMEOUT)
        {
            // Give the stderr reader a moment to collect the crash report.
            thread::sleep(Duration::from_millis(200));
            return Err(match host.last_error() {
                Some(line) => anyhow!("{err:#}: {line}"),
                None => err,
            });
        }
        Ok(host)
    }

    /// Runs a manifest command; blocks until the extension answers.
    pub fn execute(&self, command: &str) -> Result<Value> {
        let params = serde_json::json!({ "command": command });
        self.connection
            .request(protocol::EXECUTE_COMMAND, params, COMMAND_TIMEOUT)
    }

    pub fn send_event(&self, kind: &str, path: &str) {
        let _ = self
            .connection
            .notify(protocol::EVENT, protocol::event_params(kind, path));
    }

    pub fn respond(&self, id: Value, result: Result<Value, (i64, String)>) {
        let _ = self.connection.respond(id, result);
    }

    /// The last non-empty stderr line, to explain a failure.
    pub fn last_error(&self) -> Option<String> {
        let tail = self.connection.stderr_tail();
        tail.iter()
            .rev()
            .find(|l| !l.trim().is_empty())
            .map(|l| l.trim().chars().take(300).collect())
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.connection.notify(protocol::SHUTDOWN, Value::Null);
        let Some(mut child) = self.child.get_mut().ok().and_then(Option::take) else {
            return;
        };
        // Off the caller's thread: dropping happens on the UI thread when a window closes.
        let _ = thread::Builder::new().name("ext-shutdown".into()).spawn(move || {
            thread::sleep(SHUTDOWN_GRACE);
            if matches!(child.try_wait(), Ok(None)) {
                let _ = child.kill();
            }
            let _ = child.wait();
        });
    }
}

fn open_log(path: &Path) -> Option<fs::File> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).ok()?;
    }
    OpenOptions::new().create(true).append(true).open(path).ok()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// Starts a real process: a shell script that answers `initialize` and one command.
    #[test]
    fn starts_initializes_and_runs_a_command() {
        let dir = std::env::temp_dir().join(format!("pi-ext-host-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        // Reads two framed requests (ids 1 and 2) and answers each with a fixed result.
        let script = r#"#!/bin/sh
answer() {
  read -r header; read -r blank
  len=$(printf '%s' "$header" | tr -dc '0-9')
  body=$(dd bs=1 count="$len" 2>/dev/null)
  printf 'Content-Length: %d\r\n\r\n%s' "${#2}" "$2"
}
answer 1 '{"jsonrpc":"2.0","id":1,"result":{}}'
answer 2 '{"jsonrpc":"2.0","id":2,"result":"done"}'
echo "bye" >&2
"#;
        fs::write(dir.join("ext.sh"), script).unwrap();
        let extension = Extension {
            manifest: Manifest::parse("id = \"t\"\ncommand = \"/bin/sh\"\nargs = [\"ext.sh\"]").unwrap(),
            dir: dir.clone(),
        };
        let log = dir.join("logs/t.log");
        let closed = Arc::new(Mutex::new(false));
        let flag = closed.clone();
        let handler: Handler = Arc::new(move |m| {
            if m == Incoming::Closed {
                *flag.lock().unwrap() = true;
            }
        });

        let host = Host::start(&extension, &PathBuf::from("/tmp"), Some(&log), handler).unwrap();
        assert_eq!(host.execute("t.run").unwrap(), Value::from("done"));
        for _ in 0..200 {
            if *closed.lock().unwrap() && host.last_error().is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(host.last_error().as_deref(), Some("bye"));
        drop(host);
        assert_eq!(fs::read_to_string(&log).unwrap(), "bye\n");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn reports_a_missing_program() {
        let extension = Extension {
            manifest: Manifest::parse("id = \"t\"\ncommand = \"./does-not-exist\"").unwrap(),
            dir: std::env::temp_dir(),
        };
        let err = Host::start(&extension, Path::new("/tmp"), None, Arc::new(|_| {}))
            .err()
            .unwrap();
        assert!(format!("{err:#}").contains("does-not-exist"), "{err:#}");
    }
}
