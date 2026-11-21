//! One project window: project tree, editor tabs, status bar and pickers.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::component::diff::Diff;
use gpui_kit::component::input::Editor;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::tree::{TreeEvent, TreeState};
use gpui_kit::component::{
    ActiveTheme as _, WindowExt as _, h_flex, h_resizable, resizable_panel, v_flex, v_resizable,
};
use gpui_kit::*;
use search::FileIndex;

use crate::actions::*;
use crate::config::{AppConfig, ProjectSettings};
use crate::editor::CenterTab;
use crate::palette::{Palette, PaletteMode, PaletteSource};
use crate::project_tree;
use crate::terminal::TerminalView;

mod action_handlers;
mod chrome;
mod database_panel;
mod editors;
mod files;
mod git;
mod git_log;
mod git_panel;
mod mcp_tools;
mod navigation;
mod sidebar;
mod terminal_panel;
mod unsaved;

const MAX_RECENT_FILES: usize = 50;

/// All open project windows, used for routing MCP calls and focusing
/// an already open project.
#[derive(Default)]
pub struct Workspaces(Vec<(AnyWindowHandle, WeakEntity<Workspace>)>);

impl Global for Workspaces {}

impl Workspaces {
    fn register(window: AnyWindowHandle, workspace: WeakEntity<Workspace>, cx: &mut App) {
        let list = &mut cx.default_global::<Workspaces>().0;
        list.retain(|(_, ws)| ws.upgrade().is_some());
        list.push((window, workspace));
    }

    /// Open workspaces with their windows.
    pub fn live(cx: &App) -> Vec<(AnyWindowHandle, Entity<Workspace>)> {
        cx.try_global::<Workspaces>().map_or_default(|w| {
            w.0.iter()
                .filter_map(|(h, ws)| Some((*h, ws.upgrade()?)))
                .collect()
        })
    }
}

/// Marker type keying one "language server failed" notification per server.
struct ServerFailureNotice;

/// A notification change queued outside a window context, applied on render.
enum Notice {
    /// Replaces the failure notification of a language server.
    ServerFailed {
        server_id: String,
        notification: Box<Notification>,
    },
    /// Removes it once the server is running again.
    ServerRecovered {
        server_id: String,
    },
    Error(String),
}

impl Notice {
    fn show(self, window: &mut Window, cx: &mut App) {
        match self {
            Self::ServerFailed {
                server_id,
                notification,
            } => {
                window.remove_notification1::<ServerFailureNotice>(server_id.clone(), cx);
                window.push_notification((*notification).id1::<ServerFailureNotice>(server_id), cx);
            }
            Self::ServerRecovered { server_id } => {
                window.remove_notification1::<ServerFailureNotice>(server_id, cx);
            }
            Self::Error(message) => window.push_notification(Notification::error(message), cx),
        }
    }
}

/// Tool window shown below the editor.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BottomPanel {
    Terminal,
    GitLog,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LeftPanel {
    Project,
    Commit,
    Database,
}

pub struct Workspace {
    root: PathBuf,
    focus_handle: FocusHandle,
    tree: Entity<TreeState>,
    expanded: HashSet<String>,
    index: Arc<FileIndex>,
    tabs: Vec<CenterTab>,
    active_tab: Option<usize>,
    /// Most recently used first, relative to `root`.
    recent_files: Vec<String>,
    show_project_view: bool,
    left_panel: LeftPanel,
    git: Option<git::GitState>,
    git_log: Option<git_log::GitLogState>,
    project: ProjectSettings,
    /// Languages whose "install a language server" banner was dismissed.
    dismissed_language_hints: HashSet<String>,
    lsp: crate::language::LspStore,
    nav: navigation::NavHistory,
    /// Suppresses history recording while navigating programmatically.
    nav_paused: bool,
    databases: crate::database::DatabaseStore,
    /// Notifications raised outside of a window context, shown on next render.
    pending_notices: Vec<Notice>,
    /// Console to open once its data source finishes connecting.
    pending_console: Option<(String, String, bool)>,
    terminals: Vec<Entity<TerminalView>>,
    active_terminal: usize,
    show_bottom: bool,
    bottom_panel: BottomPanel,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub fn new(root: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let expanded = HashSet::new();
        let items = project_tree::build_items(&root, "", &expanded);
        let tree = cx.new(|cx| TreeState::new(cx).items(items));
        let tree_subscription = cx.subscribe(&tree, |this, _, event: &TreeEvent, cx| {
            match event {
                TreeEvent::Expanded(id) => this.expanded.insert(id.to_string()),
                TreeEvent::Collapsed(id) => this.expanded.remove(&id.to_string()),
            };
            this.refresh_tree(cx);
        });

        let project = ProjectSettings::load(&root);
        let lsp = crate::language::LspStore::new(root.clone());
        let databases = crate::database::DatabaseStore::load(&root);
        let mut workspace = Self {
            root,
            project,
            dismissed_language_hints: HashSet::new(),
            lsp,
            databases,
            pending_console: None,
            pending_notices: Vec::new(),
            nav: navigation::NavHistory::default(),
            nav_paused: false,
            focus_handle: cx.focus_handle(),
            tree,
            expanded,
            index: Arc::default(),
            tabs: Vec::new(),
            active_tab: None,
            recent_files: Vec::new(),
            show_project_view: true,
            left_panel: LeftPanel::Project,
            git: None,
            git_log: None,
            terminals: Vec::new(),
            active_terminal: 0,
            show_bottom: false,
            bottom_panel: BottomPanel::Terminal,
            _subscriptions: vec![tree_subscription],
        };
        workspace.focus_handle.focus(window, cx);
        workspace.reindex(cx);
        workspace.init_git(window, cx);
        workspace.guard_window_close(window, cx);
        workspace.start_lsp_event_loop(cx);
        workspace.preload_language_servers(cx);
        Workspaces::register(window.window_handle(), cx.entity().downgrade(), cx);
        workspace
    }

    pub fn root(&self) -> &PathBuf {
        &self.root
    }

    pub fn project_settings(&self) -> &ProjectSettings {
        &self.project
    }

    /// Applies and persists project settings, re-indexing when excludes change.
    pub fn update_project_settings(
        &mut self,
        update: impl FnOnce(&mut ProjectSettings),
        cx: &mut Context<Self>,
    ) {
        let before = self.project.clone();
        update(&mut self.project);
        if self.project == before {
            return;
        }
        if let Err(err) = self.project.save(&self.root) {
            eprintln!("saving project settings: {err:#}");
        }
        if self.project.exclude != before.exclude {
            self.reindex(cx);
        }
        cx.notify();
    }

    fn project_name(&self) -> String {
        self.root
            .file_name()
            .map_or_default(|n| n.to_string_lossy().into_owned())
    }

    /// Rebuilds the file index in the background.
    pub(crate) fn reindex(&self, cx: &mut Context<Self>) {
        let root = self.root.clone();
        let exclude = self.project.exclude.clone();
        cx.spawn(async move |this, cx| {
            let index = cx
                .background_spawn(async move { FileIndex::build_excluding(root, &exclude) })
                .await;
            this.update(cx, |this, cx| {
                this.index = Arc::new(index);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn refresh_tree(&mut self, cx: &mut Context<Self>) {
        let items = project_tree::build_items(&self.root, "", &self.expanded);
        self.tree.update(cx, |tree, cx| tree.set_items(items, cx));
    }

    fn notify_error(&self, message: String, window: &mut Window, cx: &mut App) {
        window.push_notification(Notification::error(message), cx);
    }

    /// Runs a palette-selected action once focus is back in the workspace.
    pub fn dispatch_deferred(
        &mut self,
        action: Box<dyn Action>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.active() {
            Some(tab) => tab.state.update(cx, |state, cx| state.focus(window, cx)),
            None => self.focus_handle.focus(window, cx),
        }
        window.defer(cx, move |window, cx| window.dispatch_action(action, cx));
    }

    fn open_palette(&mut self, mode: PaletteMode, window: &mut Window, cx: &mut Context<Self>) {
        self.open_palette_with(mode, Vec::new(), window, cx);
    }

    fn open_palette_with(
        &mut self,
        mode: PaletteMode,
        branches: Vec<vcs::Branch>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut recent_files = self.recent_files.clone();
        // Like JetBrains, the current file is listed last in Recent Files.
        if mode == PaletteMode::RecentFiles && !recent_files.is_empty() {
            recent_files.rotate_left(1);
        }
        let source = PaletteSource {
            workspace: cx.entity().downgrade(),
            root: self.root.clone(),
            index: self.index.clone(),
            recent_files,
            recent_projects: cx.global::<AppConfig>().0.recent_projects.clone(),
            branches,
            exclude: self.project.exclude.clone(),
        };
        Palette::open(mode, source, window, cx);
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        for notice in std::mem::take(&mut self.pending_notices) {
            notice.show(window, cx);
        }
        let (background, foreground) = (cx.theme().background, cx.theme().foreground);
        let content = match self.active_tab.and_then(|ix| self.tabs.get(ix)) {
            Some(CenterTab::Editor(tab)) => Some(
                v_flex()
                    .size_full()
                    .children(self.render_language_hint(tab, cx))
                    .child(
                        div()
                            .key_context(CODE_EDITOR_CONTEXT)
                            .flex_1()
                            .overflow_hidden()
                            .child(Editor::new(&tab.state).bordered(false).size_full()),
                    )
                    .into_any_element(),
            ),
            Some(CenterTab::Diff(tab)) => Some(
                v_flex()
                    .size_full()
                    .child(self.render_diff_toolbar(tab, cx))
                    .child(
                        div().flex_1().overflow_hidden().child(
                            Diff::new(&tab.state)
                                .line_number(true)
                                .syntax_highlight(true)
                                .size_full(),
                        ),
                    )
                    .into_any_element(),
            ),
            Some(CenterTab::Console { console, .. }) => Some(console.clone().into_any_element()),
            Some(CenterTab::Settings) => Some(
                div()
                    .size_full()
                    .child(crate::settings::render(cx.entity().downgrade(), cx))
                    .into_any_element(),
            ),
            None => None,
        };
        let center = match content {
            Some(content) => v_flex()
                .size_full()
                .child(self.render_tabs(cx))
                .child(div().flex_1().overflow_hidden().child(content))
                .into_any_element(),
            None => self.render_empty_state(window, cx).into_any_element(),
        };

        let mut body = h_resizable("workspace");
        if self.show_project_view {
            body = body.child(
                resizable_panel()
                    .size(px(260.))
                    .size_range(px(160.)..px(600.))
                    .fixed()
                    .child(self.render_sidebar(cx)),
            );
        }
        let bottom = match self.bottom_panel {
            _ if !self.show_bottom => None,
            BottomPanel::Terminal if !self.terminals.is_empty() => {
                Some(self.render_terminal_panel(cx).into_any_element())
            }
            BottomPanel::GitLog => self.render_git_log(cx),
            BottomPanel::Terminal => None,
        };
        let center = match bottom {
            Some(bottom) => v_resizable("center")
                .child(resizable_panel().child(center))
                .child(
                    resizable_panel()
                        .size(px(280.))
                        .size_range(px(80.)..px(1600.))
                        .child(bottom),
                )
                .into_any_element(),
            None => center,
        };
        body = body.child(resizable_panel().child(center));

        let root = v_flex()
            .key_context(WORKSPACE_CONTEXT)
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(background)
            .text_color(foreground)
            .child(self.render_title_bar(window, cx))
            .child(
                h_flex()
                    .flex_1()
                    .overflow_hidden()
                    .child(self.render_activity_bar(cx))
                    .child(div().flex_1().h_full().overflow_hidden().child(body)),
            )
            .child(self.render_status_bar(cx));
        self.register_actions(root, cx)
    }
}
