//! One project window: project tree, editor tabs, status bar and pickers.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::component::diff::Diff;
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
mod branches;
mod breadcrumbs;
mod busy_terminals;
mod chrome;
mod close_confirm;
mod code_nav;
mod conflicts;
mod conflicts_view;
mod database_panel;
mod debug;
mod docker_panel;
mod docker_view;
mod editors;
mod file_clipboard;
mod files;
mod fs_watch;
mod git;
mod git_log;
mod git_panel;
mod insights_advice;
mod insights_panel;
mod insights_parts;
mod insights_view;
mod mcp_code_tools;
mod mcp_tools;
mod navigation;
mod preview_view;
mod push_dialog;
mod replace_in_files;
mod run_configs;
mod session_state;
mod sidebar;
mod terminal_panel;
mod tree_select;
mod unsaved;
mod usages_panel;
mod window_tabs;

pub(crate) use insights_panel::clear_insights;
pub(crate) use unsaved::{request_quit, store_recovery};

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
    Usages,
    Debug,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LeftPanel {
    Project,
    Commit,
    Database,
    Docker,
    Insights,
}

pub struct Workspace {
    root: PathBuf,
    focus_handle: FocusHandle,
    tree: Entity<TreeState>,
    /// The project view (root row + tree): keeps ⌘X / ⌘C / ⌘V in the tree's key context
    /// when focus is on the root row or comes back from a tree context menu.
    project_focus: FocusHandle,
    expanded: HashSet<String>,
    /// Files waiting for Paste in the project tree.
    file_clipboard: Option<crate::file_ops::FileClipboard>,
    tree_selection: crate::tree_selection::TreeSelection,
    conflicts: conflicts::ConflictsState,
    /// The tree row the keyboard cursor was last put on by us; any other row
    /// selected in the tree (arrow keys) replaces the multi-selection.
    tree_cursor: Option<String>,
    index: Arc<FileIndex>,
    tabs: Vec<CenterTab>,
    active_tab: Option<usize>,
    /// Most recently used first, relative to `root`.
    recent_files: Vec<String>,
    show_project_view: bool,
    /// The project root row is collapsed: the tree under it is hidden.
    root_collapsed: bool,
    /// The project root row is selected (Paste goes into the root); only while nothing
    /// in the tree is selected.
    root_selected: bool,
    left_panel: LeftPanel,
    git: Option<git::GitState>,
    git_log: Option<git_log::GitLogState>,
    /// The open Branches popup under the title bar's branch widget.
    branches_popup: Option<(Entity<crate::branches_popup::BranchesPopup>, Subscription)>,
    /// Open editors, read by word completion.
    open_buffers: crate::word_completion::OpenBuffers,
    /// Run configurations (user + detected) and the one Run (⌃R) starts.
    run_configs: Vec<run::RunConfig>,
    selected_run: Option<String>,
    breakpoints: debug::Breakpoints,
    debug: Option<debug::DebugSession>,
    /// Docker panel state; created the first time the panel opens.
    docker: Option<docker_panel::DockerState>,
    /// Claude Session Insights; created the first time the panel opens.
    insights: Option<insights_panel::InsightsState>,
    usages: Option<usages_panel::UsagesState>,
    project: ProjectSettings,
    /// Languages whose "install a language server" banner was dismissed.
    dismissed_language_hints: HashSet<String>,
    lsp: crate::language::LspStore,
    /// Latest diagnostics per file, as published by language servers.
    diagnostics: std::collections::HashMap<PathBuf, Vec<lsp::lsp_types::Diagnostic>>,
    nav: navigation::NavHistory,
    /// Suppresses history recording while navigating programmatically.
    nav_paused: bool,
    /// Workspace state as last written to `.power-idea/workspace.toml`.
    saved_session: Option<crate::session::Session>,
    databases: crate::database::DatabaseStore,
    /// Notifications raised outside of a window context, shown on next render.
    pending_notices: Vec<Notice>,
    /// Console to open once its data source finishes connecting.
    pending_console: Option<(String, String, bool)>,
    /// Watches the project folder for changes made outside the IDE.
    watcher: Option<notify::RecommendedWatcher>,
    terminals: Vec<Entity<TerminalView>>,
    /// Result of the running-terminals check for the close in progress (`busy_terminals`).
    terminals_checked: Option<bool>,
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
        let tree_cursor_subscription = cx.observe(&tree, |this, tree, cx| this.follow_tree_cursor(&tree, cx));

        let project = ProjectSettings::load(&root);
        let lsp = crate::language::LspStore::new(root.clone());
        let databases = crate::database::DatabaseStore::load(&root);
        let mut workspace = Self {
            root,
            project,
            dismissed_language_hints: HashSet::new(),
            lsp,
            diagnostics: Default::default(),
            databases,
            pending_console: None,
            pending_notices: Vec::new(),
            nav: navigation::NavHistory::default(),
            nav_paused: false,
            saved_session: None,
            focus_handle: cx.focus_handle(),
            tree,
            project_focus: cx.focus_handle(),
            expanded,
            file_clipboard: None,
            tree_selection: Default::default(),
            conflicts: Default::default(),
            tree_cursor: None,
            index: Arc::default(),
            tabs: Vec::new(),
            active_tab: None,
            recent_files: Vec::new(),
            show_project_view: true,
            root_collapsed: false,
            root_selected: false,
            left_panel: LeftPanel::Project,
            git: None,
            git_log: None,
            branches_popup: None,
            open_buffers: Default::default(),
            run_configs: Vec::new(),
            selected_run: None,
            breakpoints: Default::default(),
            debug: None,
            docker: None,
            insights: None,
            usages: None,
            watcher: None,
            terminals: Vec::new(),
            terminals_checked: None,
            active_terminal: 0,
            show_bottom: false,
            bottom_panel: BottomPanel::Terminal,
            _subscriptions: vec![tree_subscription, tree_cursor_subscription],
        };
        workspace.focus_handle.focus(window, cx);
        workspace.reindex(cx);
        workspace.init_git(window, cx);
        workspace.start_watching(window, cx);
        workspace.load_run_configs(false, window, cx);
        workspace.guard_window_close(window, cx);
        workspace.start_lsp_event_loop(cx);
        workspace.preload_language_servers(cx);
        Workspaces::register(window.window_handle(), cx.entity().downgrade(), cx);
        let this = cx.entity().downgrade();
        window.defer(cx, move |window, cx| {
            this.update(cx, |ws, cx| ws.restore_session(window, cx)).ok();
        });
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
        let root = self.root.clone();
        self.tree_selection.retain(|rel| root.join(rel).exists());
        let cursor = self.tree_cursor.clone();
        // `set_items` drops the tree's own selection; keep the keyboard cursor in place.
        self.tree.update(cx, |tree, cx| {
            tree.set_items(items, cx);
            let ix = cursor.and_then(|id| tree.index_of(&id.into()));
            tree.set_selected_index(ix, cx);
        });
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
        let mut source = self.palette_source(cx);
        // Like JetBrains, the current file is listed last in Recent Files.
        if mode == PaletteMode::RecentFiles && !source.recent_files.is_empty() {
            source.recent_files.rotate_left(1);
        }
        Palette::open(mode, source, window, cx);
    }

    fn palette_source(&self, cx: &Context<Self>) -> PaletteSource {
        PaletteSource {
            workspace: cx.entity().downgrade(),
            root: self.root.clone(),
            index: self.index.clone(),
            recent_files: self.recent_files.clone(),
            recent_projects: cx.global::<AppConfig>().0.recent_projects.clone(),
            exclude: self.project.exclude.clone(),
            locations: Vec::new(),
            symbol_search: None,
            run_configs: Vec::new(),
        }
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        for notice in std::mem::take(&mut self.pending_notices) {
            notice.show(window, cx);
        }
        let (background, foreground) = (cx.theme().background, cx.theme().foreground);
        let svg = self.active_svg_image(cx);
        let content = match self.active_tab.and_then(|ix| self.tabs.get(ix)) {
            Some(CenterTab::Editor(tab)) => Some(
                v_flex()
                    .size_full()
                    .children(self.render_language_hint(tab, cx))
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .child(self.render_editor_area(tab, svg, cx)),
                    )
                    .into_any_element(),
            ),
            Some(CenterTab::Image(tab)) => Some(self.render_image_tab(tab, cx)),
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
            Some(CenterTab::Conflicts) => Some(self.render_conflicts_tab(cx)),
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
            BottomPanel::Usages => self.render_usages(cx),
            BottomPanel::Debug => self.render_debug_panel(cx),
            BottomPanel::Terminal => None,
        };
        let with_bottom = |top: AnyElement, bottom: AnyElement| {
            v_resizable("center")
                .child(resizable_panel().child(top))
                .child(
                    resizable_panel()
                        .size(px(280.))
                        .size_range(px(80.)..px(1600.))
                        .child(bottom),
                )
                .into_any_element()
        };
        let body = match bottom {
            Some(bottom) if cx.global::<AppConfig>().0.bottom_panel_full_width => with_bottom(
                body.child(resizable_panel().child(center)).into_any_element(),
                bottom,
            ),
            Some(bottom) => body
                .child(resizable_panel().child(with_bottom(center, bottom)))
                .into_any_element(),
            None => body.child(resizable_panel().child(center)).into_any_element(),
        };

        let root = v_flex()
            .key_context(WORKSPACE_CONTEXT)
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(background)
            .text_color(foreground)
            .child(self.render_title_bar(window, cx))
            .children(self.render_window_tabs(window, cx))
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
