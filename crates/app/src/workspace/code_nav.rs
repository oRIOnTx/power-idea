//! Language-server navigation and refactoring: Find Usages (⌥F7), Go to
//! Class / Symbol (⌘O / ⌘⌥O), File Structure (⌘F12), Rename (⇧F6) and
//! Reformat Code (⌥⌘L). Requests block, so they run in the background.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;
use lsp::LanguageServer;
use lsp::lsp_types::{self, Position, TextEdit, Uri, request};
use lsp::nav::{self, SymbolItem};

use super::Workspace;
use crate::config::AppConfig;
use crate::editor::CenterTab;
use crate::language::text_document;
use crate::palette::{LocationItem, Palette, PaletteMode, SymbolSearch};
use crate::usages::{Usage, UsageGroup, group_usages};

/// Results shown by Go to Symbol / Class.
const MAX_SYMBOLS: usize = 200;

/// The active editor's document as its language server sees it.
#[derive(Clone)]
struct ActiveDoc {
    server: Arc<LanguageServer>,
    uri: Uri,
    path: PathBuf,
    position: Position,
}

impl ActiveDoc {
    fn position_params(&self) -> lsp_types::TextDocumentPositionParams {
        lsp_types::TextDocumentPositionParams {
            text_document: text_document(&self.uri),
            position: self.position,
        }
    }
}

fn symbol_location(symbol: SymbolItem, path: &Path) -> LocationItem {
    LocationItem {
        label: symbol.name,
        detail: symbol.detail,
        kind: nav::kind_label(symbol.kind),
        indent: symbol.depth,
        path: symbol.path.unwrap_or_else(|| path.to_path_buf()),
        line: symbol.position.line,
        column: symbol.position.character,
    }
}

/// Usages grouped by file with their source lines; files are read once each.
pub(super) fn usage_groups(locations: Vec<lsp_types::Location>, root: &Path) -> Vec<UsageGroup> {
    let usages = locations
        .into_iter()
        .filter_map(|location| {
            let (start, end) = (location.range.start, location.range.end);
            Some(Usage {
                path: lsp::uri_to_path(&location.uri).ok()?,
                line: start.line,
                start: start.character,
                end: if end.line == start.line {
                    end.character
                } else {
                    start.character
                },
            })
        })
        .collect();
    group_usages(usages, root, |path| {
        std::fs::read_to_string(path).map_or_default(|text| text.lines().map(str::to_string).collect())
    })
}

impl Workspace {
    /// The active document's server, if it supports `capability`; otherwise explains why not.
    fn active_doc(
        &self,
        capability: &str,
        what: &str,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<ActiveDoc> {
        let Some(tab) = self.active() else {
            window.push_notification(Notification::info(format!("{what}: open a file first")), cx);
            return None;
        };
        let Some(lsp) = &tab.lsp else {
            let text = format!("{what} needs a language server for {}", tab.language);
            window.push_notification(Notification::info(text), cx);
            return None;
        };
        if !lsp.server.supports(capability) {
            let text = format!("{} does not support {what}", lsp.server.name());
            window.push_notification(Notification::info(text), cx);
            return None;
        }
        Some(ActiveDoc {
            server: lsp.server.clone(),
            uri: lsp.uri.clone(),
            path: tab.path.clone(),
            position: tab.state.read(cx).cursor_position(),
        })
    }

    /// Moves to a location, recording the jump in Back/Forward history.
    pub fn jump_to(
        &mut self,
        path: &Path,
        line: u32,
        column: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_path(path, Some((line as usize, column as usize)), window, cx);
    }

    /// Runs `request` in the background and hands its result to `done` on the UI thread.
    fn lsp_task<T: Send + 'static>(
        &self,
        what: &'static str,
        request: impl FnOnce() -> Result<T> + Send + 'static,
        done: impl FnOnce(&mut Self, T, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            let result = cx.background_spawn(async move { request() }).await;
            this.update_in(cx, |ws, window, cx| match result {
                Ok(value) => done(ws, value, window, cx),
                Err(err) => window.push_notification(
                    Notification::error(format!("{err:#}")).title(format!("{what} failed")),
                    cx,
                ),
            })
            .ok();
        })
        .detach();
    }

    fn open_locations(
        &mut self,
        mode: PaletteMode,
        locations: Vec<LocationItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut source = self.palette_source(cx);
        source.locations = locations;
        Palette::open(mode, source, window, cx);
    }

    pub(super) fn find_usages(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(doc) = self.active_doc("referencesProvider", "Find Usages", window, cx) else {
            return;
        };
        let title = self
            .active()
            .and_then(|tab| {
                let text = tab.state.read(cx).value();
                let line = text.lines().nth(doc.position.line as usize)?.to_string();
                nav::word_at(&line, doc.position.character)
            })
            .unwrap_or_else(|| "symbol".into());
        let root = self.root.clone();
        self.lsp_task(
            "Find Usages",
            move || {
                let params = lsp_types::ReferenceParams {
                    text_document_position: doc.position_params(),
                    context: lsp_types::ReferenceContext {
                        include_declaration: true,
                    },
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default(),
                };
                let locations = doc.server.request::<request::References>(params)?;
                Ok(usage_groups(locations.unwrap_or_default(), &root))
            },
            move |ws, groups, window, cx| match groups.as_slice() {
                [] => window.push_notification(Notification::info("No usages found"), cx),
                // A single usage is a plain jump, like JetBrains.
                [only] if only.usages.len() == 1 => {
                    let usage = &only.usages[0];
                    ws.jump_to(&only.path.clone(), usage.line, usage.column, window, cx);
                }
                _ => ws.show_usages(title, groups, window, cx),
            },
            window,
            cx,
        );
    }

    pub(super) fn file_structure(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(doc) = self.active_doc("documentSymbolProvider", "File Structure", window, cx) else {
            return;
        };
        self.lsp_task(
            "File Structure",
            move || {
                let params = lsp_types::DocumentSymbolParams {
                    text_document: text_document(&doc.uri),
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default(),
                };
                let symbols = doc.server.request::<request::DocumentSymbolRequest>(params)?;
                Ok(nav::document_symbols(symbols)
                    .into_iter()
                    .map(|s| symbol_location(s, &doc.path))
                    .collect::<Vec<_>>())
            },
            |ws, symbols, window, cx| {
                if symbols.is_empty() {
                    window.push_notification(Notification::info("No symbols in this file"), cx);
                } else {
                    ws.open_locations(PaletteMode::FileStructure, symbols, window, cx);
                }
            },
            window,
            cx,
        );
    }

    /// A running server that can search workspace symbols, preferring the active file's.
    pub(super) fn symbol_server(&self) -> Option<Arc<LanguageServer>> {
        let active = self
            .active()
            .and_then(|tab| tab.lsp.as_ref())
            .map(|l| l.server.clone());
        let running = self
            .lsp
            .statuses()
            .into_iter()
            .filter_map(|(id, _, _)| self.lsp.running(&id));
        active
            .into_iter()
            .chain(running)
            .find(|server| server.supports("workspaceSymbolProvider"))
    }

    /// ⌘O (classes, modules, …) and ⌘⌥O (every symbol).
    pub(super) fn goto_symbol(&mut self, classes_only: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(server) = self.symbol_server() else {
            let text = "Go to Symbol needs a running language server with workspace symbols";
            return window.push_notification(Notification::info(text), cx);
        };
        let root = self.root.clone();
        let search: SymbolSearch = Arc::new(move |query: &str| {
            let params = lsp_types::WorkspaceSymbolParams {
                query: query.to_string(),
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            };
            let response = server
                .request::<request::WorkspaceSymbolRequest>(params)
                .ok()
                .flatten();
            let mut symbols: Vec<LocationItem> = nav::workspace_symbols(response)
                .into_iter()
                .filter(|s| !classes_only || nav::is_type_like(s.kind))
                .map(|s| symbol_location(s, &root))
                .collect();
            // Project code before libraries (gems, crates) the server also indexes.
            symbols.sort_by_key(|s| !s.path.starts_with(&root));
            symbols.truncate(MAX_SYMBOLS);
            symbols
        });
        let mut source = self.palette_source(cx);
        source.symbol_search = Some(search);
        let mode = if classes_only {
            PaletteMode::GotoClass
        } else {
            PaletteMode::GotoSymbol
        };
        Palette::open(mode, source, window, cx);
    }

    pub(super) fn rename_symbol(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(doc) = self.active_doc("renameProvider", "Rename", window, cx) else {
            return;
        };
        let word = self
            .active()
            .and_then(|tab| {
                let text = tab.state.read(cx).value();
                let line = text.lines().nth(doc.position.line as usize)?.to_string();
                nav::word_at(&line, doc.position.character)
            })
            .unwrap_or_default();
        self.prompt_name("Rename", &word, window, cx, move |ws, name, window, cx| {
            let name = name.trim().to_string();
            if name.is_empty() {
                anyhow::bail!("Name is empty");
            }
            let doc = doc.clone();
            ws.lsp_task(
                "Rename",
                move || {
                    let params = lsp_types::RenameParams {
                        text_document_position: doc.position_params(),
                        new_name: name,
                        work_done_progress_params: Default::default(),
                    };
                    match doc.server.request::<request::Rename>(params)? {
                        Some(edit) => nav::workspace_edit_files(edit),
                        None => Ok(Vec::new()),
                    }
                },
                |ws, files, window, cx| ws.apply_rename(files, window, cx),
                window,
                cx,
            );
            Ok(())
        });
    }

    /// Applies rename edits: open editors are updated in place (and stay unsaved),
    /// other files are rewritten on disk. Files outside the project are skipped.
    fn apply_rename(
        &mut self,
        files: Vec<(PathBuf, Vec<TextEdit>)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (mut changed, mut skipped, mut errors) = (0, 0, Vec::new());
        for (path, edits) in files {
            if !path.starts_with(&self.root) {
                skipped += 1;
                continue;
            }
            match self.apply_file_edits(&path, &edits, window, cx) {
                Ok(()) => changed += 1,
                Err(err) => errors.push(format!("{}: {err:#}", path.display())),
            }
        }
        let notification = match (changed, errors.is_empty()) {
            (0, true) => Notification::info("Nothing to rename here"),
            (_, true) => {
                let mut text = format!("Renamed in {changed} file{}", if changed == 1 { "" } else { "s" });
                if skipped > 0 {
                    text.push_str(&format!("; skipped {skipped} outside the project"));
                }
                Notification::success(text)
            }
            (_, false) => Notification::error(errors.join("\n")).title("Rename was incomplete"),
        };
        window.push_notification(notification, cx);
        self.reindex(cx);
    }

    fn apply_file_edits(
        &mut self,
        path: &Path,
        edits: &[TextEdit],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let open = self.tabs.iter().find_map(|tab| match tab {
            CenterTab::Editor(tab) if tab.path == path => Some(tab.state.clone()),
            _ => None,
        });
        match open {
            Some(state) => {
                let text = nav::apply_text_edits(&state.read(cx).value(), edits)?;
                state.update(cx, |state, cx| {
                    let cursor = state.cursor_position();
                    state.set_value(text, window, cx);
                    state.set_cursor_position(cursor, window, cx);
                });
                Ok(())
            }
            None => {
                let text = std::fs::read_to_string(path).context("reading")?;
                std::fs::write(path, nav::apply_text_edits(&text, edits)?).context("writing")
            }
        }
    }

    pub(super) fn reformat_code(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(doc) = self.active_doc("documentFormattingProvider", "Reformat Code", window, cx) else {
            return;
        };
        let tab_size = self
            .project
            .tab_size
            .unwrap_or(cx.global::<AppConfig>().0.tab_size);
        let path = doc.path.clone();
        self.lsp_task(
            "Reformat Code",
            move || {
                let params = lsp_types::DocumentFormattingParams {
                    text_document: text_document(&doc.uri),
                    options: lsp_types::FormattingOptions {
                        tab_size: tab_size as u32,
                        insert_spaces: true,
                        ..Default::default()
                    },
                    work_done_progress_params: Default::default(),
                };
                doc.server.request::<request::Formatting>(params)
            },
            move |ws, edits, window, cx| {
                // `null` means the server has no formatter for this file (e.g. ruby-lsp without RuboCop).
                let Some(edits) = edits else {
                    let text = "The language server has no formatter for this file";
                    return window.push_notification(Notification::info(text), cx);
                };
                if edits.is_empty() {
                    return window.push_notification(Notification::info("Already formatted"), cx);
                }
                if let Err(err) = ws.apply_file_edits(&path, &edits, window, cx) {
                    window.push_notification(Notification::error(format!("Reformat Code: {err:#}")), cx);
                }
            },
            window,
            cx,
        );
    }
}
