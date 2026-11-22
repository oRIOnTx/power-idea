//! Code navigation: language servers per editor tab (completion, hover,
//! Cmd+Click / Cmd+B to definition, diagnostics) and Back/Forward history.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::component::input::{EditorState, Position, RopeExt as _, ShowDocumentHandler};
use gpui_kit::*;
use lsp::lsp_types::{self, notification};

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::notification::{Notification, NotificationType};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};

use super::{Notice, Workspace};
use crate::config::AppConfig;
use crate::editor::{self, CenterTab, EditorTab};
use crate::language::{self, DocumentLsp, LspEvent, TabLsp};
use crate::word_completion::WordCompletion;

const MAX_HISTORY: usize = 100;

#[derive(Clone, PartialEq)]
struct NavLocation {
    path: PathBuf,
    line: u32,
    column: u32,
}

/// JetBrains-style Back (Cmd+[) / Forward (Cmd+]) history.
#[derive(Default)]
pub struct NavHistory {
    back: Vec<NavLocation>,
    forward: Vec<NavLocation>,
}

impl Workspace {
    /// Processes language server events for the lifetime of the workspace.
    pub(super) fn start_lsp_event_loop(&self, cx: &mut Context<Self>) {
        let events = self.lsp.events();
        cx.spawn(async move |this, cx| {
            while let Some(batch) = events.next_batch().await {
                let alive = this.update(cx, |ws, cx| {
                    batch.into_iter().for_each(|e| ws.handle_lsp_event(e, cx));
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn handle_lsp_event(&mut self, event: LspEvent, cx: &mut Context<Self>) {
        match event {
            LspEvent::Started {
                server_id,
                result,
                fallback,
            } => {
                self.lsp.mark_started(&server_id, &result, fallback.is_some());
                match result {
                    Ok(_) => {
                        match &fallback {
                            Some(fallback) => self.report_fallback(&server_id, fallback, cx),
                            None => self.pending_notices.push(Notice::ServerRecovered {
                                server_id: server_id.clone(),
                            }),
                        }
                        for ix in 0..self.tabs.len() {
                            self.attach_language_server(ix, cx);
                        }
                    }
                    Err(reason) => self.report_server_failure(&server_id, &reason, cx),
                }
                cx.notify();
            }
            LspEvent::Progress {
                server_id,
                token,
                title,
                percentage,
                done,
            } => {
                self.lsp
                    .update_progress(&server_id, token, title, percentage, done);
                cx.notify();
            }
            LspEvent::Exited { server_id } => {
                if let Some(reason) = self.lsp.mark_exited(&server_id) {
                    for tab in self.tabs.iter_mut() {
                        if let CenterTab::Editor(tab) = tab
                            && tab.lsp.as_ref().is_some_and(|l| l.server.name() == server_id)
                        {
                            tab.lsp = None;
                        }
                    }
                    self.report_server_failure(&server_id, &reason, cx);
                }
                cx.notify();
            }
            LspEvent::Diagnostics { path, diagnostics } => {
                // Kept for every file (open or not) so MCP clients can ask for them.
                if diagnostics.is_empty() {
                    self.diagnostics.remove(&path);
                } else {
                    self.diagnostics.insert(path.clone(), diagnostics.clone());
                }
                let Some(tab) = self
                    .tabs
                    .iter()
                    .filter_map(CenterTab::editor)
                    .find(|t| t.path == path)
                else {
                    return;
                };
                tab.state.update(cx, |state, cx| {
                    let text = state.text().clone();
                    if let Some(set) = state.diagnostics_mut() {
                        set.reset(&text);
                        set.extend(diagnostics);
                    }
                    cx.notify();
                });
            }
        }
    }

    /// Starts servers for the languages the project uses, so indexing begins
    /// before the first file is opened.
    pub(super) fn preload_language_servers(&mut self, cx: &mut Context<Self>) {
        let servers = cx.global::<AppConfig>().0.language_servers.clone();
        for language in language::project_languages(&self.root) {
            if let Some((id, config)) = language::server_for(&servers, language)
                && crate::settings::find_in_path(&config.command).is_some()
            {
                self.lsp.ensure_started(id, config, cx);
            }
        }
    }

    /// Restarts a server after a failure (status bar click).
    pub(super) fn restart_language_server(&mut self, server_id: &str, cx: &mut Context<Self>) {
        let Some(config) = cx
            .global::<AppConfig>()
            .0
            .language_servers
            .get(server_id)
            .cloned()
        else {
            return;
        };
        self.lsp.reset(server_id);
        self.lsp.ensure_started(server_id, &config, cx);
        cx.notify();
    }

    fn report_server_failure(&mut self, server_id: &str, reason: &str, cx: &mut Context<Self>) {
        let mut message = reason.to_string();
        if let Some(hint) = language::failure_hint(reason) {
            message.push_str("\n\n");
            message.push_str(&hint);
        }
        let title = format!("{server_id} language server is not running");
        let notification = self.server_notice(NotificationType::Error, title, message, server_id, reason, cx);
        self.pending_notices.push(Notice::ServerFailed {
            server_id: server_id.to_string(),
            notification: Box::new(notification),
        });
    }

    /// The server runs, but without the project's environment.
    fn report_fallback(&mut self, server_id: &str, fallback: &language::Fallback, cx: &mut Context<Self>) {
        let title = format!("{server_id} language server · fallback mode");
        let message = fallback.message(server_id);
        let notification = self.server_notice(
            NotificationType::Warning,
            title,
            message,
            server_id,
            &fallback.reason,
            cx,
        );
        self.pending_notices.push(Notice::ServerFailed {
            server_id: server_id.to_string(),
            notification: Box::new(notification),
        });
    }

    /// A sticky notification about a server, with "Fix in Terminal" when a
    /// repair command is known.
    fn server_notice(
        &self,
        kind: NotificationType,
        title: String,
        message: String,
        server_id: &str,
        reason: &str,
        cx: &mut Context<Self>,
    ) -> Notification {
        let notification = Notification::new().with_type(kind).title(title).autohide(false);
        let Some(command) = language::fix_commands(server_id, reason) else {
            return notification.message(message);
        };
        let workspace = cx.entity().downgrade();
        notification.content(move |_, _, cx| {
            let (workspace, command) = (workspace.clone(), command.clone());
            v_flex()
                .gap_2()
                .child(div().text_sm().child(message.clone()))
                .child(
                    h_flex().child(
                        Button::new("fix-in-terminal")
                            .small()
                            .primary()
                            .label("Fix in Terminal")
                            .on_click(move |_, window, cx| {
                                workspace
                                    .update(cx, |ws, cx| ws.open_terminal_with_input(&command, window, cx))
                                    .ok();
                            }),
                    ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("Opens a terminal with the commands typed in; press Enter to run them."),
                )
                .into_any_element()
        })
    }

    /// Binds an editor tab to its language server, starting the server if needed.
    pub(super) fn attach_language_server(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(CenterTab::Editor(tab)) = self.tabs.get(ix) else {
            return;
        };
        if tab.lsp.is_some() {
            return;
        }
        let language = editor::canonical_language(&tab.language).to_string();
        let servers = cx.global::<AppConfig>().0.language_servers.clone();
        let Some((server_id, config)) = language::server_for(&servers, &language) else {
            return;
        };
        if crate::settings::find_in_path(&config.command).is_none() {
            return; // The editor banner suggests installing it.
        }
        let Some(server) = self.lsp.running(server_id) else {
            self.lsp.ensure_started(server_id, config, cx);
            return;
        };
        let Ok(uri) = lsp::path_to_uri(&tab.path) else {
            return;
        };

        let text = tab.state.read(cx).value().to_string();
        let opened =
            server.notify::<notification::DidOpenTextDocument>(lsp_types::DidOpenTextDocumentParams {
                text_document: lsp_types::TextDocumentItem {
                    uri: uri.clone(),
                    language_id: editor::lsp_language_id(&language).to_string(),
                    version: 1,
                    text,
                },
            });
        if opened.is_err() {
            return;
        }

        let origin = language::DefinitionOrigin::default();
        let show_document = self.show_document_handler(tab.path.clone(), origin.clone(), cx);
        // The server's completion replaces the editor's provider: put word completion back under it.
        let (own, buffers, editor_language) = (
            tab.state.entity_id(),
            self.open_buffers.clone(),
            tab.language.clone(),
        );
        let replaces_completion = server.supports("completionProvider");
        tab.state.update(cx, |state: &mut EditorState, _| {
            DocumentLsp::new(server.clone(), uri.clone(), origin).install(state);
            if replaces_completion {
                WordCompletion::install(state, &editor_language, own, buffers);
            }
            state.lsp_mut().show_document = Some(show_document);
        });
        if let Some(CenterTab::Editor(tab)) = self.tabs.get_mut(ix) {
            tab.lsp = Some(TabLsp {
                server,
                uri,
                version: 1,
            });
        }
    }

    /// Opens definitions in other files in the IDE (Cmd+Click, Cmd+B).
    /// Opens definitions in other files in the IDE (Cmd+Click, Cmd+B).
    ///
    /// The editor calls this while it is being updated, so the handler must
    /// not touch the editor or the workspace synchronously: it only decides
    /// whether the editor should jump itself, and defers everything else.
    fn show_document_handler(
        &self,
        current: PathBuf,
        origin: language::DefinitionOrigin,
        cx: &mut Context<Self>,
    ) -> ShowDocumentHandler {
        let workspace = cx.entity().downgrade();
        Rc::new(move |params, window, cx| {
            let Ok(path) = lsp::uri_to_path(&params.uri) else {
                return false;
            };
            let target = params
                .selection
                .map(|r| (r.start.line as usize, r.start.character as usize));
            let from = origin.get().map(|(line, column)| NavLocation {
                path: current.clone(),
                line,
                column,
            });
            let same_file = path == current;
            let workspace = workspace.clone();
            window.defer(cx, move |window, cx| {
                workspace
                    .update(cx, |ws, cx| {
                        if let Some(from) = from {
                            ws.push_history(from);
                        }
                        if !same_file {
                            ws.nav_paused = true;
                            ws.open_path(&path, target, window, cx);
                            ws.nav_paused = false;
                        }
                    })
                    .ok();
            });
            // Same file: let the editor move its own cursor.
            !same_file
        })
    }

    /// Opens an absolute path, using a project-relative name when possible.
    pub(super) fn open_path(
        &mut self,
        path: &Path,
        position: Option<(usize, usize)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = path
            .strip_prefix(&self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned();
        self.open_file(&name, position, window, cx);
    }

    pub(super) fn lsp_did_change(&mut self, state: &Entity<EditorState>, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter_mut().find_map(|t| match t {
            CenterTab::Editor(tab) if &tab.state == state => Some(tab),
            _ => None,
        }) else {
            return;
        };
        let Some(lsp) = tab.lsp.as_mut() else { return };
        lsp.version += 1;
        let text = tab.state.read(cx).value().to_string();
        let _ = lsp.server.notify::<notification::DidChangeTextDocument>(
            lsp_types::DidChangeTextDocumentParams {
                text_document: lsp_types::VersionedTextDocumentIdentifier {
                    uri: lsp.uri.clone(),
                    version: lsp.version,
                },
                content_changes: vec![lsp_types::TextDocumentContentChangeEvent {
                    range: None,
                    range_length: None,
                    text,
                }],
            },
        );
    }

    pub(super) fn lsp_did_save(tab: &EditorTab) {
        if let Some(lsp) = &tab.lsp {
            let _ = lsp.server.notify::<notification::DidSaveTextDocument>(
                lsp_types::DidSaveTextDocumentParams {
                    text_document: language::text_document(&lsp.uri),
                    text: None,
                },
            );
        }
    }

    pub(super) fn lsp_did_close(tab: &EditorTab) {
        if let Some(lsp) = &tab.lsp {
            let _ = lsp.server.notify::<notification::DidCloseTextDocument>(
                lsp_types::DidCloseTextDocumentParams {
                    text_document: language::text_document(&lsp.uri),
                },
            );
        }
    }

    fn current_location(&self, cx: &App) -> Option<NavLocation> {
        let tab = self.active()?;
        let cursor = tab.state.read(cx).cursor_position();
        Some(NavLocation {
            path: tab.path.clone(),
            line: cursor.line,
            column: cursor.character,
        })
    }

    /// Remembers where the cursor is before a jump.
    pub(super) fn record_location(&mut self, cx: &App) {
        if let Some(location) = self.current_location(cx) {
            self.push_history(location);
        }
    }

    fn push_history(&mut self, location: NavLocation) {
        if self.nav_paused {
            return;
        }
        if self.nav.back.last() != Some(&location) {
            self.nav.back.push(location);
            if self.nav.back.len() > MAX_HISTORY {
                self.nav.back.remove(0);
            }
        }
        self.nav.forward.clear();
    }

    /// Records the current spot when opening another file or jumping to a position.
    pub(super) fn record_location_if_leaving(&mut self, rel: &str, jumps: bool, cx: &App) {
        let leaving = self.active().is_some_and(|tab| tab.rel != rel);
        if leaving || jumps {
            self.record_location(cx);
        }
    }

    pub(super) fn navigate(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let target = if forward {
            self.nav.forward.pop()
        } else {
            self.nav.back.pop()
        };
        let Some(target) = target else { return };
        if let Some(current) = self.current_location(cx) {
            if forward {
                self.nav.back.push(current);
            } else {
                self.nav.forward.push(current);
            }
        }
        // Open without recording, so the stacks above stay as they are.
        self.nav_paused = true;
        self.open_path(&target.path.clone(), None, window, cx);
        self.nav_paused = false;
        if let Some(tab) = self.active() {
            tab.state.update(cx, |state, cx| {
                let last_line = state.text().lines_len().saturating_sub(1) as u32;
                state.set_cursor_position(
                    Position::new(target.line.min(last_line), target.column),
                    window,
                    cx,
                );
            });
        }
    }
}
