//! Answers extension requests (`HostRequest`, see `docs/extensions.md`) against this window:
//! editor reads and edits, opening files, notifications, pickers, inputs and terminal runs.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::Arc;

use ::extensions::protocol::{offset_of, position_of};
use ::extensions::rpc::{INTERNAL_ERROR, INVALID_PARAMS, METHOD_NOT_FOUND};
use ::extensions::{Extension, Host, HostRequest, Level, PickItem, Position};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::EditorState;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;
use serde_json::{Value, json};

use super::Workspace;
use crate::editor::CenterTab;
use crate::palette::{Palette, PaletteMode};

/// Answers one request exactly once; dropped unanswered (a cancelled picker or input)
/// it answers `null`.
struct Reply {
    host: Arc<Host>,
    id: Option<Value>,
}

impl Reply {
    fn send(mut self, result: Result<Value, String>) {
        if let Some(id) = self.id.take() {
            self.host.respond(id, result.map_err(|err| (INTERNAL_ERROR, err)));
        }
    }
}

impl Drop for Reply {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            self.host.respond(id, Ok(Value::Null));
        }
    }
}

impl Workspace {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn handle_extension_request(
        &mut self,
        extension: &Extension,
        host: Arc<Host>,
        id: Value,
        method: &str,
        params: Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let request = match HostRequest::parse(method, params) {
            Ok(request) => request,
            Err(err) => {
                let code = if err.starts_with("unknown method") {
                    METHOD_NOT_FOUND
                } else {
                    INVALID_PARAMS
                };
                return host.respond(id, Err((code, err)));
            }
        };
        let reply = Reply { host, id: Some(id) };
        match request {
            HostRequest::Pick { items, placeholder } => {
                self.extension_pick(&items, placeholder, reply, window, cx);
            }
            HostRequest::Input { title, value } => {
                let title = title.unwrap_or_else(|| extension.name().to_string());
                self.extension_input(title, value.unwrap_or_default(), reply, window, cx);
            }
            request => reply.send(self.answer_extension_request(request, extension, window, cx)),
        }
    }

    /// Requests answered right away.
    fn answer_extension_request(
        &mut self,
        request: HostRequest,
        extension: &Extension,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Value, String> {
        match request {
            HostRequest::WorkspaceInfo {} => Ok(self.extension_workspace_info(cx)),
            HostRequest::OpenFile { path, line, column } => {
                let rel = self.project_rel(&path)?;
                if !self.root.join(&rel).is_file() {
                    return Err(format!("{rel} is not a file"));
                }
                let zero_based = |n: u32| n.saturating_sub(1) as usize;
                let position = line.map(|line| (zero_based(line), zero_based(column.unwrap_or(1))));
                self.open_file(&rel, position, window, cx);
                Ok(Value::Null)
            }
            HostRequest::ActiveEditor {} => Ok(self.extension_active_editor(cx)),
            HostRequest::Text { path } => self.extension_text(path.as_deref(), cx).map(Value::from),
            HostRequest::Edit { path, range, text } => {
                let state = self.extension_target(path.as_deref(), window, cx)?;
                state.update(cx, |state, cx| {
                    let current = state.value();
                    let range = match range {
                        Some(range) => byte_range(&current, range.start, range.end)?,
                        None => state.selected_range(),
                    };
                    // One replacement: a single ⌘Z takes the whole edit back.
                    state.set_selected_range(range, cx);
                    state.replace(text, window, cx);
                    Ok(Value::Null)
                })
            }
            HostRequest::Select { path, start, end } => {
                let state = self.extension_target(path.as_deref(), window, cx)?;
                state.update(cx, |state, cx| {
                    let range = byte_range(&state.value(), start, end.unwrap_or(start))?;
                    // Moving the cursor scrolls it into view; then the selection is applied.
                    let cursor = gpui_kit::component::input::Position::new(
                        start.line.saturating_sub(1),
                        start.column.saturating_sub(1),
                    );
                    state.set_cursor_position(cursor, window, cx);
                    state.set_selected_range(range, cx);
                    Ok(Value::Null)
                })
            }
            HostRequest::Notify {
                message,
                title,
                level,
            } => {
                let notification = match level {
                    Level::Info => Notification::info(message),
                    Level::Success => Notification::success(message),
                    Level::Warning => Notification::warning(message),
                    Level::Error => Notification::error(message),
                };
                let title = title.unwrap_or_else(|| extension.name().to_string());
                window.push_notification(notification.title(title), cx);
                Ok(Value::Null)
            }
            HostRequest::RunInTerminal { command, title, cwd } => {
                let cwd = match cwd {
                    Some(cwd) => self.root.join(self.project_rel(&cwd)?),
                    None => self.root.clone(),
                };
                if !cwd.is_dir() {
                    return Err(format!("{} is not a folder", cwd.display()));
                }
                let title = title.unwrap_or_else(|| format!("{}: {command}", extension.name()));
                self.open_run_tab(title, &command, cwd, &BTreeMap::new(), window, cx);
                Ok(Value::Null)
            }
            // Answered later through dialogs (`handle_extension_request`).
            HostRequest::Pick { .. } | HostRequest::Input { .. } => Ok(Value::Null),
        }
    }

    fn extension_workspace_info(&self, cx: &App) -> Value {
        let open_files: Vec<&str> = self
            .tabs
            .iter()
            .filter_map(CenterTab::editor)
            .map(|t| t.rel.as_str())
            .collect();
        json!({
            "root": self.root,
            "name": self.project_name(),
            "branch": self.git.as_ref().and_then(|g| g.status.branch.clone()),
            "openFiles": open_files,
            "activeFile": self.active().map(|t| t.rel.clone()),
            "unsavedFiles": self.tabs.iter().filter_map(CenterTab::editor)
                .filter(|t| t.is_dirty(cx)).map(|t| t.rel.clone()).collect::<Vec<_>>(),
        })
    }

    fn extension_active_editor(&self, cx: &App) -> Value {
        let Some(tab) = self.active() else {
            return Value::Null;
        };
        let state = tab.state.read(cx);
        let text = state.value();
        let selection = state.selected_range();
        json!({
            "path": tab.rel,
            "language": tab.language,
            "cursor": position_of(&text, state.cursor()),
            "selection": {
                "start": position_of(&text, selection.start),
                "end": position_of(&text, selection.end),
            },
            "selectedText": text.get(selection).unwrap_or_default(),
            "unsaved": tab.is_dirty(cx),
        })
    }

    /// The active editor's text, or a project file's (its open buffer when there is one).
    fn extension_text(&self, path: Option<&str>, cx: &App) -> Result<String, String> {
        let Some(path) = path else {
            let tab = self.active().ok_or("no file is open in the editor")?;
            return Ok(tab.state.read(cx).value().to_string());
        };
        let rel = self.project_rel(path)?;
        let open = self
            .tabs
            .iter()
            .filter_map(CenterTab::editor)
            .find(|t| t.rel == rel);
        match open {
            Some(tab) => Ok(tab.state.read(cx).value().to_string()),
            None => std::fs::read_to_string(self.root.join(&rel)).map_err(|e| format!("{rel}: {e}")),
        }
    }

    /// The editor to change: the active one, or `path`'s (opened when needed).
    fn extension_target(
        &mut self,
        path: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Entity<EditorState>, String> {
        let Some(path) = path else {
            return self
                .active()
                .map(|t| t.state.clone())
                .ok_or_else(|| "no file is open in the editor".into());
        };
        let rel = self.project_rel(path)?;
        let find = |ws: &Self| {
            ws.tabs
                .iter()
                .filter_map(CenterTab::editor)
                .find(|t| t.rel == rel)
                .map(|t| t.state.clone())
        };
        if let Some(state) = find(self) {
            return Ok(state);
        }
        if !self.root.join(&rel).is_file() {
            return Err(format!("{rel} is not a file"));
        }
        self.open_file(&rel, None, window, cx);
        find(self).ok_or_else(|| format!("cannot open {rel}"))
    }

    fn extension_pick(
        &mut self,
        items: &[PickItem],
        placeholder: Option<String>,
        reply: Reply,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if items.is_empty() {
            return reply.send(Ok(Value::Null));
        }
        let mut source = self.palette_source(cx);
        source.choices = items
            .iter()
            .map(|item| (item.label().to_string(), item.detail().map(str::to_string)))
            .collect();
        source.placeholder = placeholder.map(Into::into);
        source.on_choice = Some(Box::new(move |ix, _, _| reply.send(Ok(json!(ix)))));
        Palette::open(PaletteMode::Choice, source, window, cx);
    }

    fn extension_input(
        &mut self,
        title: String,
        value: String,
        reply: Reply,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let reply = RefCell::new(Some(reply));
        self.prompt_name(title, &value, window, cx, move |_, text, _, _| {
            if let Some(reply) = reply.borrow_mut().take() {
                reply.send(Ok(json!(text)));
            }
            Ok(())
        });
    }
}

/// Byte range between two 1-based positions (in either order).
fn byte_range(text: &str, a: Position, b: Position) -> Result<std::ops::Range<usize>, String> {
    let offset = |p: Position| offset_of(text, p).ok_or_else(|| format!("line {} is past the end", p.line));
    let (a, b) = (offset(a)?, offset(b)?);
    Ok(a.min(b)..a.max(b))
}
