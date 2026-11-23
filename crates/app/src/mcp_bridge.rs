//! Connects the MCP server thread to the UI: tool calls are queued, executed
//! on the main thread against the right workspace, and answered via a channel.

use std::collections::VecDeque;
use std::fs;
use std::io::Read as _;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::task::{Poll, Waker};
use std::time::Duration;

use anyhow::{Context as _, Result};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;
use mcp::Tool;
use serde_json::{Value, json};

use crate::config::{AppConfig, Config};
use crate::workspace::Workspaces;

const CALL_TIMEOUT: Duration = Duration::from_secs(60);

struct Call {
    name: String,
    args: Value,
    reply: mpsc::Sender<Result<String, String>>,
}

#[derive(Default)]
struct Inbox {
    calls: VecDeque<Call>,
    waker: Option<Waker>,
}

#[derive(Clone, Default)]
struct Bridge(Arc<Mutex<Inbox>>);

impl Bridge {
    async fn next_batch(&self) -> Vec<Call> {
        std::future::poll_fn(|cx| {
            let mut inbox = self.0.lock().unwrap();
            if inbox.calls.is_empty() {
                inbox.waker = Some(cx.waker().clone());
                return Poll::Pending;
            }
            Poll::Ready(inbox.calls.drain(..).collect())
        })
        .await
    }
}

impl mcp::Handler for Bridge {
    fn tools(&self) -> Vec<Tool> {
        tools()
    }

    fn call(&self, name: &str, args: Value) -> Result<String, String> {
        let (reply, response) = mpsc::channel();
        {
            let mut inbox = self.0.lock().unwrap();
            inbox.calls.push_back(Call {
                name: name.to_string(),
                args,
                reply,
            });
            if let Some(waker) = inbox.waker.take() {
                waker.wake();
            }
        }
        response
            .recv_timeout(CALL_TIMEOUT)
            .unwrap_or_else(|_| Err("the IDE did not answer in time".into()))
    }
}

/// Server state shown to the user ("Copy Claude MCP Command").
pub struct McpStatus {
    pub port: Option<u16>,
    pub error: Option<String>,
}

impl Global for McpStatus {}

fn token_path() -> Option<PathBuf> {
    Config::dir().map(|dir| dir.join("mcp-token"))
}

/// Loads the bearer token, creating a random one (mode 0600) on first run.
fn load_or_create_token() -> Result<String> {
    let path = token_path().context("no config directory")?;
    if let Ok(token) = fs::read_to_string(&path)
        && token.trim().len() >= 32
    {
        return Ok(token.trim().to_string());
    }
    let mut bytes = [0u8; 32];
    fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    fs::create_dir_all(path.parent().unwrap())?;
    fs::write(&path, &token)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(token)
}

/// Replaces the token; the running server keeps the old one until restart.
pub fn regenerate_token() -> Result<()> {
    if let Some(path) = token_path()
        && path.exists()
    {
        fs::remove_file(path)?;
    }
    load_or_create_token().map(|_| ())
}

pub fn start(cx: &mut App) {
    let config = cx.global::<AppConfig>().0.clone();
    if !config.mcp_enabled {
        cx.set_global(McpStatus {
            port: None,
            error: Some("MCP server is disabled in config.toml".into()),
        });
        return;
    }

    let bridge = Bridge::default();
    let started = load_or_create_token()
        .and_then(|token| Ok(mcp::serve(config.mcp_port, token, Arc::new(bridge.clone()))?));
    match started {
        Ok(addr) => cx.set_global(McpStatus {
            port: Some(addr.port()),
            error: None,
        }),
        Err(err) => {
            eprintln!("MCP server: {err:#}");
            cx.set_global(McpStatus {
                port: None,
                error: Some(format!("{err:#}")),
            });
            return;
        }
    }

    cx.spawn(async move |cx| {
        loop {
            let calls = bridge.next_batch().await;
            cx.update(|cx| {
                for call in calls {
                    dispatch(call, cx);
                }
            });
        }
    })
    .detach();
}

fn dispatch(call: Call, cx: &mut App) {
    let Call { name, args, reply } = call;
    let reply_tx = reply.clone();
    let send = move |result| {
        let _ = reply_tx.send(result);
    };

    let workspaces = Workspaces::live(cx);
    if name == "list_projects" {
        let projects: Vec<Value> = workspaces
            .iter()
            .map(|(_, ws)| ws.read(cx).mcp_project_info(cx))
            .collect();
        return send(serde_json::to_string_pretty(&json!(projects)).map_err(|e| e.to_string()));
    }

    // Explicit project, else the focused window, else the only/first one.
    let target = if let Some(project) = args.get("project").and_then(Value::as_str) {
        let wanted = PathBuf::from(project);
        let wanted = wanted.canonicalize().unwrap_or(wanted);
        workspaces
            .iter()
            .find(|(_, ws)| ws.read(cx).root() == &wanted)
            .cloned()
    } else {
        let active = cx.active_window();
        workspaces
            .iter()
            .find(|(handle, _)| Some(*handle) == active)
            .or_else(|| workspaces.first())
            .cloned()
    };
    let Some((handle, workspace)) = target else {
        return send(Err("no matching project is open in the IDE".into()));
    };

    let result = handle.update(cx, |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.mcp_call(
                &name,
                &args,
                Box::new(move |r| {
                    let _ = reply.send(r);
                }),
                window,
                cx,
            );
        });
    });
    if result.is_err() {
        send(Err("project window was closed".into()));
    }
}

/// Copies the `claude mcp add` command (with the token) to the clipboard.
pub fn copy_claude_command(window: &mut Window, cx: &mut App) {
    let status = cx.global::<McpStatus>();
    let note = match (status.port, load_or_create_token()) {
        (Some(port), Ok(token)) => {
            let command = format!(
                "claude mcp add --transport http power-idea http://127.0.0.1:{port}/mcp \
                 --header \"Authorization: Bearer {token}\""
            );
            cx.write_to_clipboard(ClipboardItem::new_string(command));
            Notification::success("Paste it in a terminal to connect Claude Code.")
                .title("MCP command copied")
        }
        (None, _) => {
            Notification::error(status.error.clone().unwrap_or_default()).title("MCP server is not running")
        }
        (_, Err(err)) => Notification::error(format!("{err:#}")).title("MCP token unavailable"),
    };
    window.push_notification(note, cx);
}

fn schema(properties: Value, required: &[&str]) -> Value {
    let mut properties = properties;
    properties["project"] = json!({
        "type": "string",
        "description": "Absolute project root. Defaults to the focused IDE window.",
    });
    json!({ "type": "object", "properties": properties, "required": required })
}

fn tools() -> Vec<Tool> {
    let path = json!({ "type": "string", "description": "File path relative to the project root (or absolute inside it)." });
    vec![
        Tool {
            name: "list_projects",
            description: "List projects open in the IDE (one per window).",
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        Tool {
            name: "get_project_info",
            description: "Root, name, git branch and editor state of the project.",
            input_schema: schema(json!({}), &[]),
        },
        Tool {
            name: "get_open_files",
            description: "Files open in editor tabs, which one is active and whether it has unsaved changes.",
            input_schema: schema(json!({}), &[]),
        },
        Tool {
            name: "get_active_editor",
            description: "The file the user is looking at: path, language, 1-based cursor position and selected text.",
            input_schema: schema(json!({}), &[]),
        },
        Tool {
            name: "get_file_text",
            description: "Text of a project file as shown in the IDE, including unsaved edits of open buffers.",
            input_schema: schema(json!({ "path": path }), &["path"]),
        },
        Tool {
            name: "find_files",
            description: "Fuzzy-find project files by name or path (like the IDE's Go to File), best matches first.",
            input_schema: schema(
                json!({ "query": { "type": "string" }, "limit": { "type": "integer", "default": 50 } }),
                &["query"],
            ),
        },
        Tool {
            name: "list_files",
            description: "List project files (respecting .gitignore), paginated.",
            input_schema: schema(
                json!({ "offset": { "type": "integer", "default": 0 }, "limit": { "type": "integer", "default": 500 } }),
                &[],
            ),
        },
        Tool {
            name: "find_in_files",
            description: "Plain-text search across project files (respecting .gitignore). Returns path, 1-based line/column and the line.",
            input_schema: schema(
                json!({
                    "query": { "type": "string" },
                    "case_sensitive": { "type": "boolean", "default": false },
                    "whole_word": { "type": "boolean", "default": false },
                    "limit": { "type": "integer", "default": 200 },
                }),
                &["query"],
            ),
        },
        Tool {
            name: "git_status",
            description: "Current branch, upstream, ahead/behind and changed files.",
            input_schema: schema(json!({}), &[]),
        },
        Tool {
            name: "git_diff",
            description: "Unified diff of local changes against HEAD, for one file or the whole project.",
            input_schema: schema(json!({ "path": path }), &[]),
        },
        Tool {
            name: "git_log",
            description: "Recent commits: short hash, author, date, subject.",
            input_schema: schema(json!({ "limit": { "type": "integer", "default": 30 } }), &[]),
        },
        Tool {
            name: "get_diagnostics",
            description: "Errors and warnings reported by language servers (1-based positions), for one file or all files that have any.",
            input_schema: schema(json!({ "path": path }), &[]),
        },
        Tool {
            name: "find_symbol",
            description: "Find classes, methods, functions… by name via the language server (like Go to Symbol). Project symbols first.",
            input_schema: schema(
                json!({ "query": { "type": "string" }, "limit": { "type": "integer", "default": 50 } }),
                &["query"],
            ),
        },
        Tool {
            name: "find_usages",
            description: "Usages of the symbol at a 1-based line/column of a file, via the language server. Returns path, line, column and the line text.",
            input_schema: schema(
                json!({ "path": path, "line": { "type": "integer" }, "column": { "type": "integer" } }),
                &["path", "line", "column"],
            ),
        },
        Tool {
            name: "get_file_structure",
            description: "Outline of a file (classes, methods, … with nesting depth and 1-based lines). The file must be open in the IDE (see open_file).",
            input_schema: schema(json!({ "path": path }), &["path"]),
        },
        Tool {
            name: "open_file",
            description: "Open a file in the IDE editor, optionally at a 1-based line and column, and focus the window.",
            input_schema: schema(
                json!({ "path": path, "line": { "type": "integer" }, "column": { "type": "integer" } }),
                &["path"],
            ),
        },
        Tool {
            name: "show_diff",
            description: "Show the IDE's side-by-side diff of a changed file to the user.",
            input_schema: schema(json!({ "path": path }), &["path"]),
        },
    ]
}
