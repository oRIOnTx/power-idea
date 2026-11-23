//! Debugging for any language through the Debug Adapter Protocol (`crates/dap`):
//! a run configuration's `debug` section picks the adapter (rdbg, lldb-dap,
//! debugpy, dlv). The session runs the build step, starts the adapter, sends
//! breakpoints, and follows stops, stepping, variables and output.

mod breakpoints;
mod panel;

use std::collections::{HashMap, HashSet};
use std::process::Command;
use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use dap::model::{Scope, StackFrame, Variable};
use dap::{AdapterKind, DapEvent, DebugAdapter, DebugTarget, LaunchPlan};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::*;
use run::RunConfig;
use serde_json::{Value, json};

pub use breakpoints::Breakpoints;

use super::{BottomPanel, Workspace};
use crate::config::AppConfig;
use crate::inbox::Inbox;

/// Console lines kept per session.
const CONSOLE_LIMIT: usize = 5000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DebugStatus {
    Starting(String),
    Running,
    Paused { thread: i64, reason: String },
    Ended(String),
}

pub struct DebugSession {
    pub name: String,
    kind: AdapterKind,
    adapter: Option<Arc<DebugAdapter>>,
    pub status: DebugStatus,
    frames: Vec<StackFrame>,
    selected_frame: usize,
    scopes: Vec<Scope>,
    /// Loaded children per variables reference.
    children: HashMap<i64, Vec<Variable>>,
    expanded: HashSet<i64>,
    console: Vec<String>,
    /// rdbg stops at the program start; that first stop is skipped.
    skipped_entry: bool,
    evaluate: Entity<InputState>,
    _evaluate_events: Subscription,
}

impl DebugSession {
    pub fn is_active(&self) -> bool {
        !matches!(self.status, DebugStatus::Ended(_))
    }

    fn paused_thread(&self) -> Option<i64> {
        match self.status {
            DebugStatus::Paused { thread, .. } => Some(thread),
            _ => None,
        }
    }

    fn log(&mut self, text: &str) {
        for line in text.trim_end_matches('\n').split('\n') {
            self.console.push(line.to_string());
        }
        let excess = self.console.len().saturating_sub(CONSOLE_LIMIT);
        self.console.drain(..excess);
    }
}

/// The command that starts lldb-dap: on PATH, or through `xcrun` on macOS.
fn lldb_dap_command() -> Option<Vec<String>> {
    if crate::settings::find_in_path("lldb-dap").is_some() {
        return Some(vec!["lldb-dap".into()]);
    }
    let found = Command::new("xcrun").args(["-f", "lldb-dap"]).output().ok()?;
    found
        .status
        .success()
        .then(|| vec!["xcrun".into(), "lldb-dap".into()])
}

/// Checks that the adapter is installed; returns lldb-dap's command for `Lldb`.
fn check_adapter(kind: AdapterKind) -> Result<Vec<String>> {
    let missing = || {
        anyhow::anyhow!(
            "{} is not installed. Install it with: {}",
            kind.label(),
            kind.install_hint()
        )
    };
    match kind {
        AdapterKind::Lldb => lldb_dap_command().ok_or_else(missing),
        AdapterKind::Ruby => crate::settings::find_in_path("rdbg")
            .map(|_| Vec::new())
            .ok_or_else(missing),
        AdapterKind::Go => crate::settings::find_in_path("dlv")
            .map(|_| Vec::new())
            .ok_or_else(missing),
        AdapterKind::Python => {
            let ok = Command::new("python3")
                .args(["-c", "import debugpy"])
                .output()
                .is_ok_and(|o| o.status.success());
            if ok { Ok(Vec::new()) } else { Err(missing()) }
        }
    }
}

/// Runs the build step in a login shell; returns its output on failure.
fn build(command: &str, target: &DebugTarget) -> Result<()> {
    let output = Command::new(&target.shell)
        .args(["-l", "-c", command])
        .current_dir(&target.cwd)
        .envs(&target.env)
        .output()
        .with_context(|| format!("running {command}"))?;
    if !output.status.success() {
        let text = String::from_utf8_lossy(&output.stderr);
        let lines: Vec<&str> = text.lines().collect();
        let tail = &lines[lines.len().saturating_sub(20)..];
        bail!("{command} failed:\n{}", tail.join("\n"));
    }
    Ok(())
}

/// Everything before the session can be configured: build, start, initialize.
fn start_adapter(
    target: &DebugTarget,
    build_step: Option<&str>,
    events: Inbox<DapEvent>,
) -> Result<(Arc<DebugAdapter>, LaunchPlan)> {
    let lldb_dap = check_adapter(target.kind)?;
    if let Some(command) = build_step {
        build(command, target)?;
    }
    let plan = dap::plan(target, dap::free_port()?, &lldb_dap)?;
    let adapter = DebugAdapter::start(&plan.adapter, Arc::new(move |event| events.push(event)))?;
    adapter.request(
        "initialize",
        json!({
            "clientID": "power-idea", "clientName": "Power IDEA", "adapterID": target.kind.label(),
            "linesStartAt1": true, "columnsStartAt1": true, "pathFormat": "path",
            "supportsVariableType": true, "supportsRunInTerminalRequest": false,
        }),
    )?;
    Ok((adapter, plan))
}

impl Workspace {
    /// Debug (⌃D): the selected run configuration under its debugger.
    pub(in crate::workspace) fn debug_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let config = self
            .selected_run
            .as_ref()
            .and_then(|name| self.run_configs.iter().find(|c| &c.name == name))
            .cloned();
        match config {
            Some(config) => self.debug_config(config, window, cx),
            None => self.choose_run_config(window, cx),
        }
    }

    pub fn debug_config(&mut self, config: RunConfig, window: &mut Window, cx: &mut Context<Self>) {
        let Some(config) = self.resolve_run_config(&config, window, cx) else {
            return;
        };
        let Some(spec) = config.debug.clone() else {
            let text = format!(
                "{} has no debugger. Add a [config.debug] section with adapter = \"ruby\" | \"lldb\" | \"python\" | \"go\" (Run → Edit Configurations)",
                config.name
            );
            return window.push_notification(Notification::info(text), cx);
        };
        let Some(kind) = AdapterKind::parse(&spec.adapter) else {
            let text = format!("Unknown debug adapter \"{}\" in {}", spec.adapter, config.name);
            return window.push_notification(Notification::error(text), cx);
        };
        self.stop_debug(cx);
        self.selected_run = Some(config.name.clone());
        let shell = Some(cx.global::<AppConfig>().0.terminal_shell.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| std::env::var("SHELL").ok())
            .unwrap_or_else(|| "/bin/zsh".into());
        let target = DebugTarget {
            kind,
            command: config.command.clone(),
            program: spec.program.clone(),
            module: spec.module.clone(),
            args: spec.args.clone(),
            cwd: config.working_dir(&self.root),
            env: config.env.clone(),
            shell,
        };

        let evaluate = cx.new(|cx| InputState::new(window, cx).placeholder("Evaluate expression (Enter)"));
        let evaluate_events =
            cx.subscribe_in(&evaluate, window, |ws, input, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    let expression = input.read(cx).value().trim().to_string();
                    if !expression.is_empty() {
                        input.update(cx, |i, cx| i.set_value("", window, cx));
                        ws.evaluate(expression, cx);
                    }
                }
            });
        let starting = if spec.build.is_some() {
            "Building…"
        } else {
            "Starting…"
        };
        self.debug = Some(DebugSession {
            name: config.name,
            kind,
            adapter: None,
            status: DebugStatus::Starting(format!("{starting} ({})", kind.label())),
            frames: Vec::new(),
            selected_frame: 0,
            scopes: Vec::new(),
            children: HashMap::new(),
            expanded: HashSet::new(),
            console: Vec::new(),
            skipped_entry: false,
            evaluate,
            _evaluate_events: evaluate_events,
        });
        self.show_bottom = true;
        self.bottom_panel = BottomPanel::Debug;
        cx.notify();

        let events: Inbox<DapEvent> = Inbox::default();
        let inbox = events.clone();
        let build_step = spec.build;
        cx.spawn_in(window, async move |this, cx| {
            let started = cx
                .background_spawn(async move { start_adapter(&target, build_step.as_deref(), inbox) })
                .await;
            let alive = this.update_in(cx, |ws, window, cx| {
                ws.debug_started(started, events.clone(), window, cx);
            });
            if alive.is_err() {
                return;
            }
            while let Some(batch) = events.next_batch().await {
                let alive = this.update_in(cx, |ws, window, cx| {
                    for event in batch {
                        ws.handle_debug_event(event, window, cx);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn debug_started(
        &mut self,
        started: Result<(Arc<DebugAdapter>, LaunchPlan)>,
        events: Inbox<DapEvent>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.debug.as_mut() else {
            return;
        };
        match started {
            Ok((adapter, plan)) => {
                session.adapter = Some(adapter.clone());
                session.status = DebugStatus::Starting("Waiting for the program…".into());
                // launch/attach may only answer after configurationDone, so do not wait for it here.
                cx.background_spawn(async move {
                    if let Err(err) = adapter.request(plan.request, plan.arguments) {
                        events.push(DapEvent::Event {
                            event: "power-idea/launchFailed".into(),
                            body: json!({ "message": format!("{err:#}") }),
                        });
                    }
                })
                .detach();
            }
            Err(err) => {
                let message = format!("{err:#}");
                session.log(&message);
                session.status = DebugStatus::Ended("Could not start".into());
                events.close();
                window.push_notification(Notification::error(message).title("Debugger did not start"), cx);
            }
        }
        cx.notify();
    }

    fn handle_debug_event(&mut self, event: DapEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.debug.as_mut() else {
            return;
        };
        match event {
            DapEvent::ProcessOutput(line) => session.log(&line),
            DapEvent::Closed => self.end_debug("Debugger disconnected", cx),
            DapEvent::Event { event, body } => match event.as_str() {
                "initialized" => self.configure_debug(cx),
                "stopped" => {
                    let thread = body.get("threadId").and_then(Value::as_i64).unwrap_or(1);
                    if session.kind.stops_at_entry() && !session.skipped_entry {
                        session.skipped_entry = true;
                        return self.debug_request("continue", json!({ "threadId": thread }), cx);
                    }
                    let reason = body
                        .get("reason")
                        .and_then(Value::as_str)
                        .unwrap_or("paused")
                        .to_string();
                    session.status = DebugStatus::Paused { thread, reason };
                    self.load_stack(thread, window, cx);
                }
                "continued" => {
                    session.status = DebugStatus::Running;
                    self.mark_execution_line(None, cx);
                }
                "output" => {
                    if let Some(text) = dap::model::output_text(&body) {
                        session.log(&text);
                    }
                }
                "exited" => {
                    let code = body.get("exitCode").and_then(Value::as_i64).unwrap_or_default();
                    session.log(&format!("Process finished with exit code {code}"));
                }
                "terminated" => self.end_debug("Finished", cx),
                "power-idea/launchFailed" => {
                    let message = body["message"].as_str().unwrap_or("launch failed").to_string();
                    session.log(&message);
                    window.push_notification(Notification::error(message).title("Debugging failed"), cx);
                    self.end_debug("Launch failed", cx);
                }
                _ => {}
            },
        }
        cx.notify();
    }

    /// Breakpoints, exception filters, then `configurationDone`.
    fn configure_debug(&mut self, cx: &mut Context<Self>) {
        let Some(adapter) = self.debug.as_ref().and_then(|d| d.adapter.clone()) else {
            return;
        };
        if let Some(session) = self.debug.as_mut() {
            session.status = DebugStatus::Running;
        }
        let breakpoints = self.all_breakpoints();
        cx.background_spawn(async move {
            for (path, lines) in breakpoints {
                let _ = adapter.request("setBreakpoints", breakpoints::breakpoint_args(&path, &lines));
            }
            let _ = adapter.request("setExceptionBreakpoints", json!({ "filters": [] }));
            let _ = adapter.request("configurationDone", json!({}));
        })
        .detach();
    }

    /// Sends a request whose answer is not needed (stepping, continue).
    fn debug_request(&self, command: &'static str, arguments: Value, cx: &mut Context<Self>) {
        let Some(adapter) = self.debug.as_ref().and_then(|d| d.adapter.clone()) else {
            return;
        };
        cx.background_spawn(async move {
            let _ = adapter.request(command, arguments);
        })
        .detach();
    }

    fn load_stack(&mut self, thread: i64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(adapter) = self.debug.as_ref().and_then(|d| d.adapter.clone()) else {
            return;
        };
        cx.spawn_in(window, async move |this, cx| {
            let trace = cx
                .background_spawn(async move {
                    adapter.request("stackTrace", json!({ "threadId": thread, "levels": 50 }))
                })
                .await;
            this.update_in(cx, |ws, window, cx| {
                let Some(session) = ws.debug.as_mut() else { return };
                match trace {
                    Ok(body) => {
                        session.frames = dap::model::stack_frames(&body);
                        // Start at the first frame with source code, like JetBrains.
                        let first = session.frames.iter().position(|f| f.path.is_some()).unwrap_or(0);
                        ws.select_frame(first, window, cx);
                    }
                    Err(err) => session.log(&format!("stackTrace failed: {err:#}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Shows a frame: its source line in the editor and its variables.
    pub(in crate::workspace) fn select_frame(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.debug.as_mut() else {
            return;
        };
        let Some(frame) = session.frames.get(ix).cloned() else {
            return;
        };
        session.selected_frame = ix;
        session.scopes.clear();
        session.children.clear();
        session.expanded.clear();
        if let Some(path) = &frame.path {
            self.jump_to(
                path,
                frame.line.saturating_sub(1),
                frame.column.saturating_sub(1),
                window,
                cx,
            );
            self.mark_execution_line(Some((path, frame.line)), cx);
        }
        let Some(adapter) = self.debug.as_ref().and_then(|d| d.adapter.clone()) else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let loaded = cx
                .background_spawn(async move {
                    let scopes =
                        dap::model::scopes(&adapter.request("scopes", json!({ "frameId": frame.id }))?);
                    let mut children = Vec::new();
                    for scope in scopes.iter().filter(|s| !s.expensive) {
                        let body =
                            adapter.request("variables", json!({ "variablesReference": scope.reference }))?;
                        children.push((scope.reference, dap::model::variables(&body)));
                    }
                    anyhow::Ok((scopes, children))
                })
                .await;
            this.update(cx, |ws, cx| {
                let Some(session) = ws.debug.as_mut() else { return };
                match loaded {
                    Ok((scopes, children)) => {
                        for (reference, variables) in children {
                            session.expanded.insert(reference);
                            session.children.insert(reference, variables);
                        }
                        session.scopes = scopes;
                    }
                    Err(err) => session.log(&format!("variables failed: {err:#}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Expands or collapses a scope or structured variable.
    pub(in crate::workspace) fn toggle_variable(&mut self, reference: i64, cx: &mut Context<Self>) {
        let Some(session) = self.debug.as_mut() else {
            return;
        };
        if !session.expanded.remove(&reference) {
            session.expanded.insert(reference);
            if !session.children.contains_key(&reference)
                && let Some(adapter) = session.adapter.clone()
            {
                cx.spawn(async move |this, cx| {
                    let body = cx
                        .background_spawn(async move {
                            adapter.request("variables", json!({ "variablesReference": reference }))
                        })
                        .await;
                    this.update(cx, |ws, cx| {
                        if let (Some(session), Ok(body)) = (ws.debug.as_mut(), body) {
                            session.children.insert(reference, dap::model::variables(&body));
                            cx.notify();
                        }
                    })
                    .ok();
                })
                .detach();
            }
        }
        cx.notify();
    }

    /// Resume, Step Over / Into / Out while paused; Pause while running.
    pub(in crate::workspace) fn debug_step(&mut self, command: &'static str, cx: &mut Context<Self>) {
        let Some(session) = self.debug.as_mut() else {
            return;
        };
        if command == "pause" {
            if session.status == DebugStatus::Running {
                // Without a stopped thread yet, ask the adapter which threads exist.
                let Some(adapter) = session.adapter.clone() else {
                    return;
                };
                cx.background_spawn(async move {
                    let thread = adapter
                        .request("threads", json!({}))
                        .ok()
                        .and_then(|b| b.pointer("/threads/0/id").and_then(Value::as_i64))
                        .unwrap_or(1);
                    let _ = adapter.request("pause", json!({ "threadId": thread }));
                })
                .detach();
            }
            return;
        }
        let Some(thread) = session.paused_thread() else {
            return;
        };
        session.status = DebugStatus::Running;
        self.mark_execution_line(None, cx);
        self.debug_request(command, json!({ "threadId": thread }), cx);
        cx.notify();
    }

    fn evaluate(&mut self, expression: String, cx: &mut Context<Self>) {
        let Some(session) = self.debug.as_mut() else {
            return;
        };
        let Some(adapter) = session.adapter.clone() else {
            return;
        };
        let frame = session.frames.get(session.selected_frame).map(|f| f.id);
        session.log(&format!("› {expression}"));
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    adapter.request(
                        "evaluate",
                        json!({ "expression": expression, "frameId": frame, "context": "repl" }),
                    )
                })
                .await;
            this.update(cx, |ws, cx| {
                let Some(session) = ws.debug.as_mut() else { return };
                match result {
                    Ok(body) => session.log(body["result"].as_str().unwrap_or("")),
                    Err(err) => session.log(&format!("error: {err:#}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn end_debug(&mut self, status: &str, cx: &mut Context<Self>) {
        let Some(session) = self.debug.as_mut() else {
            return;
        };
        if !session.is_active() {
            return;
        }
        session.status = DebugStatus::Ended(status.into());
        session.frames.clear();
        session.scopes.clear();
        if let Some(adapter) = session.adapter.take() {
            cx.background_spawn(async move { adapter.kill() }).detach();
        }
        self.mark_execution_line(None, cx);
        cx.notify();
    }

    /// Stop: ends the debuggee and the adapter.
    pub(in crate::workspace) fn stop_debug(&mut self, cx: &mut Context<Self>) {
        let Some(adapter) = self
            .debug
            .as_ref()
            .filter(|d| d.is_active())
            .and_then(|d| d.adapter.clone())
        else {
            if self.debug.as_ref().is_some_and(DebugSession::is_active) {
                self.end_debug("Stopped", cx);
            }
            return;
        };
        cx.background_spawn(async move {
            let _ = adapter.request("disconnect", json!({ "terminateDebuggee": true }));
            adapter.kill();
        })
        .detach();
        self.end_debug("Stopped", cx);
    }

    pub(in crate::workspace) fn debug_active(&self) -> bool {
        self.debug.as_ref().is_some_and(DebugSession::is_active)
    }
}
