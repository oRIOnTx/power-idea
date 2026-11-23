//! Language server integration: starts servers per language on demand and
//! adapts them to the editor's completion, hover and definition hooks.

mod hover;
mod ruby_version;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use gpui_kit::base::input::Rope;
use gpui_kit::component::input::{
    CompletionProvider, DefinitionProvider, EditorState, HoverProvider, RopeExt as _,
};
use gpui_kit::*;
use lsp::LanguageServer;
use lsp::lsp_types::{self, Uri, request};
use serde_json::Value;

use crate::config::LanguageServerConfig;
use crate::inbox::Inbox;

/// How a server came up when the project's own environment failed.
pub struct Fallback {
    /// The original startup failure.
    pub reason: String,
    /// Another Ruby version, used because the project's one lacks the server.
    pub ruby: Option<String>,
    /// A minimal bundle replaced the project's broken one.
    pub minimal_bundle: bool,
}

impl Fallback {
    /// Notification text: what runs instead, and why.
    pub fn message(&self, server_id: &str) -> String {
        let mut parts = Vec::new();
        if let Some(version) = &self.ruby {
            let why = ruby_version::describe(&self.reason).unwrap_or_default();
            parts.push(format!("{why} {server_id} runs on Ruby {version} instead."));
        }
        if self.minimal_bundle {
            parts.push(format!(
                "The project's bundle could not be loaded, so {server_id} runs with a minimal bundle: \
                 navigation, completion and hover work for project code; Rails add-ons and gem sources \
                 need the project's gems."
            ));
        }
        format!("{}\n\nCause: {}", parts.join(" "), self.reason)
    }
}

/// Events delivered from server threads to the workspace.
pub enum LspEvent {
    Started {
        server_id: String,
        result: Result<Arc<LanguageServer>, String>,
        /// Set when the server only came up in fallback mode.
        fallback: Option<Fallback>,
    },
    Diagnostics {
        path: PathBuf,
        diagnostics: Vec<lsp_types::Diagnostic>,
    },
    /// `$/progress` from the server (indexing etc.).
    Progress {
        server_id: String,
        token: String,
        title: Option<String>,
        percentage: Option<u32>,
        done: bool,
    },
    Exited {
        server_id: String,
    },
}

/// What the status bar shows for a language server.
#[derive(Clone, Debug, PartialEq)]
pub enum ServerStatus {
    Starting,
    /// Busy with a long task such as indexing.
    Working {
        title: String,
        percentage: Option<u32>,
    },
    Ready,
    Failed(String),
}

struct ServerEntry {
    server: Option<Arc<LanguageServer>>,
    /// Running without the project's own environment (see `fallback_env`).
    fallback: bool,
    status: ServerStatus,
    /// Active `$/progress` tasks by token.
    tasks: HashMap<String, (String, Option<u32>)>,
}

/// Language servers of one project window.
pub struct LspStore {
    root: PathBuf,
    servers: HashMap<String, ServerEntry>,
    events: Inbox<LspEvent>,
}

impl LspStore {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            servers: HashMap::new(),
            events: Inbox::default(),
        }
    }

    pub fn events(&self) -> Inbox<LspEvent> {
        self.events.clone()
    }

    pub fn running(&self, server_id: &str) -> Option<Arc<LanguageServer>> {
        self.servers.get(server_id).and_then(|entry| entry.server.clone())
    }

    /// Status of every known server, sorted by id.
    pub fn statuses(&self) -> Vec<(String, ServerStatus, bool)> {
        let mut statuses: Vec<_> = self
            .servers
            .iter()
            .map(|(id, e)| (id.clone(), e.status.clone(), e.fallback))
            .collect();
        statuses.sort_by(|a, b| a.0.cmp(&b.0));
        statuses
    }

    /// Starts `server_id` in the background unless it is already known.
    pub fn ensure_started(&mut self, server_id: &str, config: &LanguageServerConfig, cx: &App) {
        if self.servers.contains_key(server_id) {
            return;
        }
        self.servers.insert(
            server_id.to_string(),
            ServerEntry {
                server: None,
                fallback: false,
                status: ServerStatus::Starting,
                tasks: HashMap::new(),
            },
        );
        let (id, root, events) = (server_id.to_string(), self.root.clone(), self.events.clone());
        let (command, args) = (config.command.clone(), config.args.clone());
        cx.background_spawn(async move {
            let notifications = events.clone();
            let handler_id = id.clone();
            let handler: lsp::NotificationHandler = Arc::new(move |method, params| {
                if let Some(event) = server_event(&handler_id, method, params) {
                    notifications.push(event);
                }
            });
            let start = |env: &[(String, String)]| {
                LanguageServer::start(&id, &command, &args, env, &root, handler.clone())
                    .map_err(|e| format!("{e:#}"))
            };
            let mut env = Vec::new();
            let mut result = start(&env);
            let mut fallback = None::<Fallback>;
            // The project's Ruby lacks the server (or isn't installed): use another Ruby.
            if let Err(reason) = &result
                && id == "ruby"
                && let Some((ruby_env, version)) = ruby_version::fallback_env(reason, &command)
            {
                env = ruby_env;
                fallback = Some(Fallback {
                    reason: reason.clone(),
                    ruby: Some(version),
                    minimal_bundle: false,
                });
                result = start(&env);
            }
            // The project's environment is broken (e.g. gems missing): retry in
            // fallback mode so navigation and completion still work.
            if let Err(reason) = &result
                && let Some(bundle_env) = fallback_env(&id, reason)
            {
                let mut retry_env = env;
                retry_env.extend(bundle_env);
                if let Ok(server) = start(&retry_env) {
                    let fallback = fallback.get_or_insert_with(|| Fallback {
                        reason: reason.clone(),
                        ruby: None,
                        minimal_bundle: false,
                    });
                    fallback.minimal_bundle = true;
                    result = Ok(server);
                }
            }
            // Report the original failure when no fallback helped.
            if let (Err(_), Some(fallback)) = (&result, fallback.take()) {
                result = Err(fallback.reason);
            }
            events.push(LspEvent::Started {
                server_id: id,
                result,
                fallback,
            });
        })
        .detach();
    }

    /// Forgets a failed server so the next `ensure_started` tries again.
    pub fn reset(&mut self, server_id: &str) {
        self.servers.remove(server_id);
    }

    pub fn mark_started(
        &mut self,
        server_id: &str,
        result: &Result<Arc<LanguageServer>, String>,
        fallback: bool,
    ) {
        let Some(entry) = self.servers.get_mut(server_id) else {
            return;
        };
        entry.fallback = fallback;
        match result {
            Ok(server) => {
                entry.server = Some(server.clone());
                entry.status = ServerStatus::Ready;
            }
            Err(reason) => entry.status = ServerStatus::Failed(reason.clone()),
        }
    }

    /// Records a crash; returns the reason to report.
    pub fn mark_exited(&mut self, server_id: &str) -> Option<String> {
        let entry = self.servers.get_mut(server_id)?;
        let server = entry.server.take()?;
        let reason = server
            .failure_summary()
            .unwrap_or_else(|| "the process exited".into());
        entry.status = ServerStatus::Failed(format!("{} stopped: {reason}", server.name()));
        entry.tasks.clear();
        Some(reason)
    }

    pub fn update_progress(
        &mut self,
        server_id: &str,
        token: String,
        title: Option<String>,
        percentage: Option<u32>,
        done: bool,
    ) {
        let Some(entry) = self.servers.get_mut(server_id) else {
            return;
        };
        if done {
            entry.tasks.remove(&token);
        } else {
            let task = entry.tasks.entry(token).or_insert_with(|| (String::new(), None));
            if let Some(title) = title {
                task.0 = title;
            }
            task.1 = percentage.or(task.1);
        }
        if entry.server.is_some() {
            entry.status = match entry.tasks.values().next() {
                Some((title, percentage)) => ServerStatus::Working {
                    title: title.clone(),
                    percentage: *percentage,
                },
                None => ServerStatus::Ready,
            };
        }
    }
}

impl Drop for LspStore {
    fn drop(&mut self) {
        self.events.close();
    }
}

/// Translates server notifications into workspace events.
fn server_event(server_id: &str, method: &str, params: Value) -> Option<LspEvent> {
    match method {
        "textDocument/publishDiagnostics" => {
            let params: lsp_types::PublishDiagnosticsParams = serde_json::from_value(params).ok()?;
            let path = lsp::uri_to_path(&params.uri).ok()?;
            Some(LspEvent::Diagnostics {
                path,
                diagnostics: params.diagnostics,
            })
        }
        "$/progress" => {
            let token = match params.get("token").unwrap_or(&Value::Null) {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            let value = params.get("value").unwrap_or(&Value::Null);
            Some(LspEvent::Progress {
                server_id: server_id.to_string(),
                token,
                title: value["title"]
                    .as_str()
                    .or(value["message"].as_str())
                    .map(str::to_string),
                percentage: value["percentage"].as_u64().map(|p| p.min(100) as u32),
                done: value["kind"] == "end",
            })
        }
        lsp::EXIT_NOTIFICATION => Some(LspEvent::Exited {
            server_id: server_id.to_string(),
        }),
        _ => None,
    }
}

/// Languages a project uses, guessed from its marker files, for preloading servers.
pub fn project_languages(root: &std::path::Path) -> Vec<&'static str> {
    let has = |name: &str| root.join(name).exists();
    let mut languages = Vec::new();
    if has("Gemfile") || has(".ruby-version") {
        languages.push("ruby");
    }
    if has("Cargo.toml") {
        languages.push("rust");
    }
    if has("tsconfig.json") || has("package.json") {
        languages.push("typescript");
    }
    if has("pyproject.toml") || has("requirements.txt") || has("setup.py") {
        languages.push("python");
    }
    if has("go.mod") {
        languages.push("go");
    }
    languages
}

/// The project's Ruby bundle cannot be loaded (missing or unbuildable gems).
fn is_bundle_failure(reason: &str) -> bool {
    [
        "Could not find",
        "GemNotFound",
        "Could not create Makefile",
        "bundle install",
        "Bundler::",
    ]
    .iter()
    .any(|marker| reason.contains(marker))
}

/// Environment for running a server without the project's own setup, when
/// that setup is what broke. For Ruby: a tiny bundle containing only
/// `ruby-lsp`, so project files are still indexed and navigable.
fn fallback_env(server_id: &str, reason: &str) -> Option<Vec<(String, String)>> {
    if server_id != "ruby" || !is_bundle_failure(reason) {
        return None;
    }
    let dir = crate::config::Config::dir()?.join("ruby-lsp-fallback");
    let gemfile = dir.join("Gemfile");
    if !dir.join("Gemfile.lock").exists() {
        std::fs::create_dir_all(&dir).ok()?;
        std::fs::write(&gemfile, "source \"https://rubygems.org\"\ngem \"ruby-lsp\"\n").ok()?;
        // Prefer locally installed gems; fall back to resolving from rubygems.org.
        let lock = |args: &[&str]| {
            std::process::Command::new("bundle")
                .args(args)
                .current_dir(&dir)
                .env("BUNDLE_GEMFILE", &gemfile)
                .output()
                .is_ok_and(|o| o.status.success())
        };
        if !lock(&["lock", "--local"]) && !lock(&["lock"]) {
            return None;
        }
    }
    Some(vec![(
        "BUNDLE_GEMFILE".into(),
        gemfile.to_string_lossy().into_owned(),
    )])
}

/// Terminal commands that repair the project environment, typed but not run.
pub fn fix_commands(server_id: &str, reason: &str) -> Option<String> {
    if server_id != "ruby" {
        return None;
    }
    if let Some(command) = ruby_version::fix_command(reason) {
        return Some(command);
    }
    if !is_bundle_failure(reason) {
        return None;
    }
    Some(if reason.contains("Makefile") || reason.contains("pg") {
        "brew install libpq && bundle config build.pg --with-pg-config=$(brew --prefix libpq)/bin/pg_config && bundle install".into()
    } else {
        "bundle install".into()
    })
}

/// A next step for well-known startup failures.
pub fn failure_hint(reason: &str) -> Option<String> {
    if let Some(problem) = ruby_version::describe(reason) {
        return Some(format!(
            "{problem} No other installed Ruby has it either; install it with Fix in Terminal."
        ));
    }
    let hint = if reason.contains("Could not create Makefile")
        || reason.contains("Could not find")
        || reason.contains("GemNotFound")
    {
        Some(
            "The project's gems are not installed. Run `bundle install` in the project (native gems like pg may need system libraries, e.g. `brew install libpq`).",
        )
    } else if reason.contains("No such file") || reason.contains("not found") {
        Some("Check the server command in Settings → Languages.")
    } else {
        None
    };
    hint.map(String::from)
}

/// Picks the configured, enabled server for an editor language.
pub fn server_for<'a>(
    servers: &'a std::collections::BTreeMap<String, LanguageServerConfig>,
    language: &str,
) -> Option<(&'a String, &'a LanguageServerConfig)> {
    servers
        .iter()
        .find(|(_, s)| s.enabled && s.languages.iter().any(|l| l == language))
}

pub fn text_document(uri: &Uri) -> lsp_types::TextDocumentIdentifier {
    lsp_types::TextDocumentIdentifier { uri: uri.clone() }
}

fn position_params(uri: &Uri, text: &Rope, offset: usize) -> lsp_types::TextDocumentPositionParams {
    lsp_types::TextDocumentPositionParams {
        text_document: text_document(uri),
        position: text.offset_to_position(offset),
    }
}

/// Normalizes definition responses to location links.
pub fn definition_links(response: Option<lsp_types::GotoDefinitionResponse>) -> Vec<lsp_types::LocationLink> {
    let to_link = |location: lsp_types::Location| lsp_types::LocationLink {
        origin_selection_range: None,
        target_uri: location.uri,
        target_range: location.range,
        target_selection_range: location.range,
    };
    match response {
        None => Vec::new(),
        Some(lsp_types::GotoDefinitionResponse::Scalar(location)) => vec![to_link(location)],
        Some(lsp_types::GotoDefinitionResponse::Array(locations)) => {
            locations.into_iter().map(to_link).collect()
        }
        Some(lsp_types::GotoDefinitionResponse::Link(links)) => links,
    }
}

/// LSP state of one open editor tab.
pub struct TabLsp {
    pub server: Arc<LanguageServer>,
    pub uri: Uri,
    pub version: i32,
}

/// Editor hooks backed by one language server for one document.
pub struct DocumentLsp {
    pub server: Arc<LanguageServer>,
    pub uri: Uri,
    triggers: Vec<String>,
    /// Position of the last definition request, shared with the
    /// show-document handler so it can record history without reading the
    /// editor (which is mid-update when the handler runs).
    origin: DefinitionOrigin,
}

/// Shared slot for the (line, column) a definition lookup started from.
pub type DefinitionOrigin = std::rc::Rc<std::cell::Cell<Option<(u32, u32)>>>;

impl DocumentLsp {
    pub fn new(server: Arc<LanguageServer>, uri: Uri, origin: DefinitionOrigin) -> Self {
        let triggers = server.completion_triggers();
        Self {
            server,
            uri,
            triggers,
            origin,
        }
    }

    /// Installs completion, hover and definition providers on `state`.
    pub fn install(self, state: &mut EditorState) {
        let this = std::rc::Rc::new(self);
        let lsp = state.lsp_mut();
        if this.server.supports("completionProvider") {
            lsp.completion_provider = Some(this.clone());
        }
        if this.server.supports("hoverProvider") {
            lsp.hover_provider = Some(this.clone());
        }
        if this.server.supports("definitionProvider") {
            lsp.definition_provider = Some(this);
        }
    }
}

impl DefinitionProvider for DocumentLsp {
    fn definitions(
        &self,
        text: &Rope,
        offset: usize,
        _: &mut Window,
        cx: &mut App,
    ) -> Task<Result<Vec<lsp_types::LocationLink>>> {
        let (server, params) = (self.server.clone(), position_params(&self.uri, text, offset));
        self.origin
            .set(Some((params.position.line, params.position.character)));
        cx.background_spawn(async move {
            let params = lsp_types::GotoDefinitionParams {
                text_document_position_params: params,
                work_done_progress_params: lsp_types::WorkDoneProgressParams::default(),
                partial_result_params: lsp_types::PartialResultParams::default(),
            };
            Ok(definition_links(
                server.request::<request::GotoDefinition>(params)?,
            ))
        })
    }
}

impl HoverProvider for DocumentLsp {
    fn hover(
        &self,
        text: &Rope,
        offset: usize,
        _: &mut Window,
        cx: &mut App,
    ) -> Task<Result<Option<lsp_types::Hover>>> {
        let (server, params) = (self.server.clone(), position_params(&self.uri, text, offset));
        cx.background_spawn(async move {
            let hover = server.request::<request::HoverRequest>(lsp_types::HoverParams {
                text_document_position_params: params,
                work_done_progress_params: lsp_types::WorkDoneProgressParams::default(),
            })?;
            Ok(hover.map(hover::tidy))
        })
    }
}

impl CompletionProvider for DocumentLsp {
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        trigger: lsp_types::CompletionContext,
        _: &mut Window,
        cx: &mut App,
    ) -> Task<Result<lsp_types::CompletionResponse>> {
        let (server, params) = (self.server.clone(), position_params(&self.uri, text, offset));
        cx.background_spawn(async move {
            let response = server.request::<request::Completion>(lsp_types::CompletionParams {
                text_document_position: params,
                work_done_progress_params: lsp_types::WorkDoneProgressParams::default(),
                partial_result_params: lsp_types::PartialResultParams::default(),
                context: Some(trigger),
            })?;
            Ok(response.unwrap_or(lsp_types::CompletionResponse::Array(Vec::new())))
        })
    }

    fn is_completion_trigger(&self, _offset: usize, new_text: &str, _: &mut App) -> bool {
        new_text.chars().any(|c| c.is_alphanumeric() || c == '_')
            || self.triggers.iter().any(|t| t == new_text)
    }
}

#[cfg(test)]
mod tests {
    use super::{LspEvent, failure_hint, server_event};
    use serde_json::json;

    #[test]
    fn parses_progress_notifications() {
        let begin =
            json!({ "token": 7, "value": { "kind": "begin", "title": "Indexing", "percentage": 12 } });
        let Some(LspEvent::Progress {
            token,
            title,
            percentage,
            done,
            ..
        }) = server_event("ruby", "$/progress", begin)
        else {
            panic!("expected progress");
        };
        assert_eq!(
            (token.as_str(), title.as_deref(), percentage, done),
            ("7", Some("Indexing"), Some(12), false)
        );
        let end = json!({ "token": "7", "value": { "kind": "end" } });
        assert!(matches!(
            server_event("ruby", "$/progress", end),
            Some(LspEvent::Progress { done: true, .. })
        ));
        assert!(server_event("ruby", "window/logMessage", json!({})).is_none());
    }

    #[test]
    fn suggests_fix_commands_for_bundle_failures() {
        let pg = "ruby-lsp exited during startup: Could not create Makefile due to some reason";
        assert!(
            super::fix_commands("ruby", pg)
                .unwrap()
                .contains("brew install libpq")
        );
        assert_eq!(
            super::fix_commands("ruby", "Could not find rails-7.1").unwrap(),
            "bundle install"
        );
        assert!(super::fix_commands("rust", pg).is_none());
        assert!(super::fix_commands("ruby", "segfault").is_none());
    }

    #[test]
    fn hints_for_missing_gems() {
        assert!(
            failure_hint("ruby-lsp exited during startup: Could not find pg-1.5.9")
                .unwrap()
                .contains("bundle install")
        );
        assert!(failure_hint("something odd").is_none());
    }
}
