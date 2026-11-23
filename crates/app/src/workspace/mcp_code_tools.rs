//! MCP tools backed by language servers (read-only): diagnostics, symbol
//! search, usages and file structure. Requests run in the background.

use std::sync::Arc;

use gpui_kit::*;
use lsp::LanguageServer;
use lsp::lsp_types::{self, DiagnosticSeverity, Position, request};
use lsp::nav;
use serde_json::{Value, json};

use super::Workspace;
use super::mcp_tools::{Reply, arg_str, arg_usize, to_text};
use crate::config::AppConfig;
use crate::editor::{self, CenterTab};
use crate::language;

type SyncResult = Result<Option<(Reply, Result<String, String>)>, Reply>;

fn severity(severity: Option<DiagnosticSeverity>) -> &'static str {
    match severity {
        Some(DiagnosticSeverity::ERROR) => "error",
        Some(DiagnosticSeverity::WARNING) => "warning",
        Some(DiagnosticSeverity::INFORMATION) => "info",
        Some(DiagnosticSeverity::HINT) => "hint",
        _ => "unknown",
    }
}

impl Workspace {
    /// The running language server for a project file: the one its open tab
    /// uses, or the configured server for its language.
    fn server_for_file(&self, rel: &str, cx: &App) -> Option<Arc<LanguageServer>> {
        let open = self
            .tabs
            .iter()
            .filter_map(CenterTab::editor)
            .find(|t| t.rel == rel);
        if let Some(lsp) = open.and_then(|t| t.lsp.as_ref()) {
            return Some(lsp.server.clone());
        }
        let language = editor::canonical_language(&editor::language_for(&self.root.join(rel))).to_string();
        let servers = &cx.global::<AppConfig>().0.language_servers;
        let (id, _) = language::server_for(servers, &language)?;
        self.lsp.running(id)
    }

    fn diagnostics_json(&self, rel: Option<&str>) -> Value {
        let mut files: Vec<Value> = self
            .diagnostics
            .iter()
            .filter_map(|(path, diagnostics)| {
                let path_rel = path.strip_prefix(&self.root).ok()?.to_string_lossy().into_owned();
                if rel.is_some_and(|r| r != path_rel) {
                    return None;
                }
                let items: Vec<Value> = diagnostics
                    .iter()
                    .map(|d| {
                        json!({
                            "line": d.range.start.line + 1,
                            "column": d.range.start.character + 1,
                            "severity": severity(d.severity),
                            "message": d.message,
                            "source": d.source,
                        })
                    })
                    .collect();
                Some(json!({ "path": path_rel, "diagnostics": items }))
            })
            .collect();
        files.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
        json!(files)
    }

    /// Language-server tools; `Err(reply)` for unknown tool names.
    pub(super) fn mcp_code_call(
        &mut self,
        name: &str,
        args: &Value,
        reply: Reply,
        cx: &mut Context<Self>,
    ) -> SyncResult {
        let path_arg = |ws: &Self| {
            arg_str(args, "path")
                .ok_or_else(|| "missing path".to_string())
                .and_then(|p| ws.project_rel(p))
        };
        match name {
            "get_diagnostics" => {
                let rel = match arg_str(args, "path").map(|p| self.project_rel(p)).transpose() {
                    Ok(rel) => rel,
                    Err(err) => return Ok(Some((reply, Err(err)))),
                };
                Ok(Some((reply, to_text(self.diagnostics_json(rel.as_deref())))))
            }
            "find_symbol" => {
                let Some(query) = arg_str(args, "query").map(str::to_string) else {
                    return Ok(Some((reply, Err("missing query".into()))));
                };
                let Some(server) = self.symbol_server() else {
                    return Ok(Some((
                        reply,
                        Err("no running language server supports symbol search".into()),
                    )));
                };
                let limit = arg_usize(args, "limit").unwrap_or(50).min(500);
                let root = self.root.clone();
                cx.background_spawn(async move {
                    let result = server
                        .request::<request::WorkspaceSymbolRequest>(lsp_types::WorkspaceSymbolParams {
                            query,
                            work_done_progress_params: lsp_types::WorkDoneProgressParams::default(),
                            partial_result_params: lsp_types::PartialResultParams::default(),
                        })
                        .map_err(|e| format!("{e:#}"))
                        .and_then(|response| {
                            let mut symbols = nav::workspace_symbols(response);
                            symbols.sort_by_key(|s| !s.path.as_ref().is_some_and(|p| p.starts_with(&root)));
                            let items: Vec<Value> = symbols
                                .into_iter()
                                .take(limit)
                                .map(|s| {
                                    let path = s.path.as_ref().map(|p| {
                                        p.strip_prefix(&root).unwrap_or(p).to_string_lossy().into_owned()
                                    });
                                    json!({
                                        "name": s.name,
                                        "kind": nav::kind_label(s.kind),
                                        "container": s.detail,
                                        "path": path,
                                        "line": s.position.line + 1,
                                        "column": s.position.character + 1,
                                    })
                                })
                                .collect();
                            to_text(json!(items))
                        });
                    reply(result);
                })
                .detach();
                Ok(None)
            }
            "find_usages" => {
                let rel = match path_arg(self) {
                    Ok(rel) => rel,
                    Err(err) => return Ok(Some((reply, Err(err)))),
                };
                let (Some(line), Some(column)) = (arg_usize(args, "line"), arg_usize(args, "column")) else {
                    return Ok(Some((
                        reply,
                        Err("line and column (1-based) are required".into()),
                    )));
                };
                let Some(server) = self.server_for_file(&rel, cx) else {
                    return Ok(Some((
                        reply,
                        Err(format!("no running language server for {rel}")),
                    )));
                };
                let (root, path) = (self.root.clone(), self.root.join(&rel));
                cx.background_spawn(async move {
                    let result = (|| -> Result<String> {
                        let params = lsp_types::ReferenceParams {
                            text_document_position: lsp_types::TextDocumentPositionParams {
                                text_document: language::text_document(&lsp::path_to_uri(&path)?),
                                position: Position::new(
                                    line.saturating_sub(1) as u32,
                                    column.saturating_sub(1) as u32,
                                ),
                            },
                            context: lsp_types::ReferenceContext {
                                include_declaration: true,
                            },
                            work_done_progress_params: lsp_types::WorkDoneProgressParams::default(),
                            partial_result_params: lsp_types::PartialResultParams::default(),
                        };
                        let locations = server.request::<request::References>(params)?.unwrap_or_default();
                        let usages: Vec<Value> = super::code_nav::usage_groups(locations, &root)
                            .into_iter()
                            .flat_map(|group| {
                                group.usages.into_iter().map(move |u| {
                                    json!({
                                        "path": group.label,
                                        "line": u.line + 1,
                                        "column": u.column + 1,
                                        "text": format!("{}{}{}", u.before, u.matched, u.after),
                                    })
                                })
                            })
                            .collect();
                        Ok(serde_json::to_string_pretty(&usages)?)
                    })();
                    reply(result.map_err(|e| format!("{e:#}")));
                })
                .detach();
                Ok(None)
            }
            "get_file_structure" => {
                let rel = match path_arg(self) {
                    Ok(rel) => rel,
                    Err(err) => return Ok(Some((reply, Err(err)))),
                };
                // Most servers only outline documents they have open.
                let open = self
                    .tabs
                    .iter()
                    .filter_map(CenterTab::editor)
                    .find(|t| t.rel == rel)
                    .and_then(|t| t.lsp.as_ref().map(|l| (l.server.clone(), l.uri.clone())));
                let Some((server, uri)) = open else {
                    let text = format!("{rel} is not open with a language server; call open_file first");
                    return Ok(Some((reply, Err(text))));
                };
                cx.background_spawn(async move {
                    let result = server
                        .request::<request::DocumentSymbolRequest>(lsp_types::DocumentSymbolParams {
                            text_document: language::text_document(&uri),
                            work_done_progress_params: lsp_types::WorkDoneProgressParams::default(),
                            partial_result_params: lsp_types::PartialResultParams::default(),
                        })
                        .map_err(|e| format!("{e:#}"))
                        .and_then(|response| {
                            let items: Vec<Value> = nav::document_symbols(response)
                                .into_iter()
                                .map(|s| {
                                    json!({
                                        "name": s.name,
                                        "kind": nav::kind_label(s.kind),
                                        "detail": s.detail,
                                        "depth": s.depth,
                                        "line": s.position.line + 1,
                                    })
                                })
                                .collect();
                            to_text(json!(items))
                        });
                    reply(result);
                })
                .detach();
                Ok(None)
            }
            _ => Err(reply),
        }
    }
}
