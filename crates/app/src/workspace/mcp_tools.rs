//! Workspace state exposed to MCP tools. Heavy work runs in the background and
//! replies through `reply` once done.

use std::path::{Component, Path};

use gpui_kit::*;
use serde_json::{Value, json};
use vcs::Repo;

use super::Workspace;
use crate::editor::CenterTab;

pub type Reply = Box<dyn FnOnce(Result<String, String>) + Send>;

fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

fn arg_usize(args: &Value, key: &str) -> Option<usize> {
    args.get(key).and_then(Value::as_u64).map(|v| v as usize)
}

fn to_text(value: Value) -> Result<String, String> {
    serde_json::to_string_pretty(&value).map_err(|e| e.to_string())
}

impl Workspace {
    /// Normalizes a tool path argument to a path relative to the project root,
    /// rejecting anything outside the project.
    fn project_rel(&self, path: &str) -> Result<String, String> {
        let path = Path::new(path);
        let rel = if path.is_absolute() {
            let root = self.root.canonicalize().unwrap_or_else(|_| self.root.clone());
            path.strip_prefix(&root)
                .or_else(|_| path.strip_prefix(&self.root))
                .map_err(|_| format!("{} is outside the project", path.display()))?
                .to_path_buf()
        } else {
            path.to_path_buf()
        };
        if rel
            .components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
        {
            return Err("path must stay inside the project".into());
        }
        Ok(rel.to_string_lossy().replace('\\', "/"))
    }

    fn repo(&self) -> Result<Repo, String> {
        self.git
            .as_ref()
            .map(|g| g.repo.clone())
            .ok_or_else(|| "project is not a git repository".into())
    }

    pub fn mcp_project_info(&self, cx: &App) -> Value {
        json!({
            "root": self.root,
            "name": self.project_name(),
            "branch": self.git.as_ref().and_then(|g| g.status.branch.clone()),
            "indexed_files": self.index.len(),
            "open_tabs": self.tabs.len(),
            "has_unsaved_changes": self.tabs.iter().any(|t| t.is_dirty(cx)),
        })
    }

    /// Runs a tool against this workspace. Tools that touch the UI need `window`.
    pub fn mcp_call(
        &mut self,
        name: &str,
        args: &Value,
        reply: Reply,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.mcp_call_sync(name, args, reply, window, cx) {
            Ok(None) => {}
            Ok(Some((reply, result))) => reply(result),
            Err(reply) => reply(Err(format!("unknown tool: {name}"))),
        }
    }

    /// Returns the reply with a ready result, `Ok(None)` if a background task
    /// will reply, or `Err(reply)` for unknown tools.
    #[allow(clippy::type_complexity)]
    fn mcp_call_sync(
        &mut self,
        name: &str,
        args: &Value,
        reply: Reply,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Option<(Reply, Result<String, String>)>, Reply> {
        let result = match name {
            "get_project_info" => to_text(self.mcp_project_info(cx)),
            "get_open_files" => {
                let active = self.active_tab;
                let files: Vec<Value> = self
                    .tabs
                    .iter()
                    .enumerate()
                    .filter_map(|(ix, tab)| {
                        let editor = tab.editor()?;
                        Some(json!({
                            "path": editor.rel,
                            "active": Some(ix) == active,
                            "unsaved_changes": editor.is_dirty(cx),
                        }))
                    })
                    .collect();
                to_text(json!(files))
            }
            "get_active_editor" => match self.active() {
                None => Err("no file is open in the editor".into()),
                Some(tab) => {
                    let state = tab.state.read(cx);
                    let cursor = state.cursor_position();
                    to_text(json!({
                        "path": tab.rel,
                        "language": tab.language,
                        "cursor": { "line": cursor.line + 1, "column": cursor.character + 1 },
                        "selected_text": state.selected_value().to_string(),
                        "unsaved_changes": tab.is_dirty(cx),
                    }))
                }
            },
            "get_file_text" => {
                let rel = match arg_str(args, "path")
                    .ok_or("missing path")
                    .map(|p| self.project_rel(p))
                {
                    Ok(Ok(rel)) => rel,
                    Ok(Err(err)) => return Ok(Some((reply, Err(err)))),
                    Err(err) => return Ok(Some((reply, Err(err.into())))),
                };
                // Prefer the open buffer: it includes unsaved edits.
                let open = self
                    .tabs
                    .iter()
                    .filter_map(CenterTab::editor)
                    .find(|t| t.rel == rel)
                    .map(|t| t.state.read(cx).value().to_string());
                match open {
                    Some(text) => Ok(text),
                    None => std::fs::read_to_string(self.root.join(&rel)).map_err(|e| format!("{rel}: {e}")),
                }
            }
            "find_files" => {
                let query = arg_str(args, "query").unwrap_or_default();
                let limit = arg_usize(args, "limit").unwrap_or(50).min(1000);
                let files: Vec<String> = self
                    .index
                    .fuzzy(query, limit)
                    .into_iter()
                    .map(|m| m.path)
                    .collect();
                to_text(json!(files))
            }
            "list_files" => {
                let offset = arg_usize(args, "offset").unwrap_or(0);
                let limit = arg_usize(args, "limit").unwrap_or(500).min(5000);
                let files: Vec<&String> = self.index.files().iter().skip(offset).take(limit).collect();
                to_text(json!({ "total": self.index.len(), "files": files }))
            }
            "find_in_files" => {
                let Some(query) = arg_str(args, "query").map(str::to_string) else {
                    return Ok(Some((reply, Err("missing query".into()))));
                };
                let options = search::TextSearchOptions {
                    case_sensitive: args
                        .get("case_sensitive")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    whole_word: args.get("whole_word").and_then(Value::as_bool).unwrap_or(false),
                    limit: arg_usize(args, "limit").unwrap_or(200).min(2000),
                };
                let root = self.root.clone();
                let exclude = self.project.exclude.clone();
                cx.background_spawn(async move {
                    let matches: Vec<Value> = search::find_in_files(&root, &query, options, &exclude)
                        .into_iter()
                        .map(|m| json!({ "path": m.path, "line": m.line, "column": m.column, "text": m.line_text }))
                        .collect();
                    reply(to_text(json!(matches)));
                })
                .detach();
                return Ok(None);
            }
            "git_status" | "git_diff" | "git_log" => {
                let repo = match self.repo() {
                    Ok(repo) => repo,
                    Err(err) => return Ok(Some((reply, Err(err)))),
                };
                let prefix = self.git.as_ref().map_or_default(|g| g.prefix.clone());
                let path = match arg_str(args, "path").map(|p| self.project_rel(p)).transpose() {
                    Ok(path) => path.map(|p| format!("{prefix}{p}")),
                    Err(err) => return Ok(Some((reply, Err(err)))),
                };
                let limit = arg_usize(args, "limit").unwrap_or(30).min(500);
                let name = name.to_string();
                cx.background_spawn(async move {
                    reply(run_git_tool(&repo, &name, path, limit));
                })
                .detach();
                return Ok(None);
            }
            "open_file" => {
                let rel = match arg_str(args, "path")
                    .ok_or("missing path".to_string())
                    .and_then(|p| self.project_rel(p))
                {
                    Ok(rel) => rel,
                    Err(err) => return Ok(Some((reply, Err(err)))),
                };
                if !self.root.join(&rel).is_file() {
                    return Ok(Some((reply, Err(format!("{rel} is not a file")))));
                }
                let line = arg_usize(args, "line").map(|l| l.saturating_sub(1));
                let column = arg_usize(args, "column").map_or(0, |c| c.saturating_sub(1));
                self.open_file(&rel, line.map(|l| (l, column)), window, cx);
                window.activate_window();
                Ok(format!("Opened {rel}"))
            }
            "show_diff" => {
                let rel = match arg_str(args, "path")
                    .ok_or("missing path".to_string())
                    .and_then(|p| self.project_rel(p))
                {
                    Ok(rel) => rel,
                    Err(err) => return Ok(Some((reply, Err(err)))),
                };
                let change = self.git.as_ref().and_then(|g| {
                    let path = g.repo_path(&rel);
                    g.status.changes.iter().find(|c| c.path == path).cloned()
                });
                match change {
                    Some(change) => {
                        self.open_change_diff(change, window, cx);
                        window.activate_window();
                        Ok(format!("Showing diff of {rel}"))
                    }
                    None => Err(format!("{rel} has no local changes")),
                }
            }
            _ => return Err(reply),
        };
        Ok(Some((reply, result)))
    }
}

fn run_git_tool(repo: &Repo, name: &str, path: Option<String>, limit: usize) -> Result<String, String> {
    let err = |e: anyhow::Error| format!("{e:#}");
    match name {
        "git_status" => {
            let status = repo.status().map_err(err)?;
            let changes: Vec<Value> = status
                .changes
                .iter()
                .map(|c| json!({ "path": c.path, "kind": format!("{:?}", c.kind), "staged": c.staged }))
                .collect();
            to_text(json!({
                "branch": status.branch,
                "upstream": status.upstream,
                "ahead": status.ahead,
                "behind": status.behind,
                "changes": changes,
            }))
        }
        "git_diff" => {
            let status = repo.status().map_err(err)?;
            let mut patch = String::new();
            for change in status
                .changes
                .iter()
                .filter(|c| path.as_ref().is_none_or(|p| &c.path == p))
            {
                patch.push_str(&repo.diff_file(change).map_err(err)?);
            }
            Ok(if patch.is_empty() {
                "No local changes".into()
            } else {
                patch
            })
        }
        "git_log" => {
            let log: Vec<Value> = repo
                .log(limit)
                .map_err(err)?
                .into_iter()
                .map(|c| json!({ "hash": c.short_hash, "author": c.author, "date": c.date, "subject": c.subject }))
                .collect();
            to_text(json!(log))
        }
        _ => Err(format!("unknown tool: {name}")),
    }
}
