//! One project window: project tree, editor tabs, status bar and pickers.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::component::diff::Diff;
use gpui_kit::component::input::{Editor, InputEvent, Position, RopeExt as _};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::list::ListItem;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::tree::{TreeEvent, TreeState, tree};
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, TitleBar, WindowExt as _, h_flex, h_resizable,
    resizable_panel, v_flex, v_resizable,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use search::FileIndex;

use crate::actions::*;
use crate::config::AppConfig;
use crate::editor::{self, CenterTab, EditorTab, LineEdit};
use crate::palette::{Palette, PaletteMode, PaletteSource};
use crate::project_tree;
use crate::style;
use crate::terminal::{Session, TerminalEvent, TerminalView};

mod git;
mod mcp_tools;

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
        cx.try_global::<Workspaces>()
            .map(|w| {
                w.0.iter()
                    .filter_map(|(h, ws)| Some((*h, ws.upgrade()?)))
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LeftPanel {
    Project,
    Commit,
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
    terminals: Vec<Entity<TerminalView>>,
    active_terminal: usize,
    show_terminal: bool,
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

        let mut workspace = Self {
            root,
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
            terminals: Vec::new(),
            active_terminal: 0,
            show_terminal: false,
            _subscriptions: vec![tree_subscription],
        };
        workspace.focus_handle.focus(window, cx);
        workspace.reindex(cx);
        workspace.init_git(window, cx);
        Workspaces::register(window.window_handle(), cx.entity().downgrade(), cx);
        workspace
    }

    pub fn root(&self) -> &PathBuf {
        &self.root
    }

    fn project_name(&self) -> String {
        self.root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// Rebuilds the file index in the background.
    fn reindex(&self, cx: &mut Context<Self>) {
        let root = self.root.clone();
        cx.spawn(async move |this, cx| {
            let index = cx.background_spawn(async move { FileIndex::build(root) }).await;
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

    /// The active tab if it is a text editor.
    fn active(&self) -> Option<&EditorTab> {
        self.active_tab
            .and_then(|ix| self.tabs.get(ix))
            .and_then(CenterTab::editor)
    }

    fn notify_error(&self, message: String, window: &mut Window, cx: &mut App) {
        window.push_notification(Notification::error(message), cx);
    }

    /// Opens `rel` (relative to the project root) and optionally moves the
    /// cursor to a 0-based (line, column).
    pub fn open_file(
        &mut self,
        rel: &str,
        position: Option<(usize, usize)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = self.root.join(rel);
        if path.is_dir() {
            return;
        }

        let ix = match self
            .tabs
            .iter()
            .position(|tab| tab.editor().is_some_and(|tab| tab.rel == rel))
        {
            Some(ix) => ix,
            None => {
                let text = match fs::read(&path) {
                    Ok(bytes) if bytes.contains(&0) => {
                        self.notify_error(format!("{rel} looks like a binary file"), window, cx);
                        return;
                    }
                    Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                    Err(err) => {
                        self.notify_error(format!("Cannot open {rel}: {err}"), window, cx);
                        return;
                    }
                };
                let language = editor::language_for(&path);
                let tab_size = cx.global::<AppConfig>().0.tab_size;
                let state = editor::new_editor_state(text.clone(), &language, tab_size, window, cx);
                let subscriptions = vec![
                    cx.subscribe(&state, |_, _, _: &InputEvent, cx| cx.notify()),
                    cx.observe(&state, |_, _, cx| cx.notify()),
                ];
                self.tabs.push(CenterTab::Editor(EditorTab {
                    path,
                    rel: rel.to_string(),
                    state,
                    language,
                    saved_text: text.into(),
                    _subscriptions: subscriptions,
                }));
                self.tabs.len() - 1
            }
        };

        self.activate_tab(ix, window, cx);
        if let Some((line, column)) = position {
            self.goto_line(line as u32, column as u32, window, cx);
        }
    }

    fn activate_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(ix) else { return };
        self.active_tab = Some(ix);
        match tab {
            CenterTab::Editor(tab) => {
                let rel = tab.rel.clone();
                tab.state.update(cx, |state, cx| state.focus(window, cx));
                self.recent_files.retain(|r| *r != rel);
                self.recent_files.insert(0, rel);
                self.recent_files.truncate(MAX_RECENT_FILES);
            }
            CenterTab::Diff(tab) => tab.state.read(cx).focus_handle(cx).focus(window, cx),
        }
        cx.notify();
    }

    pub fn goto_line(&mut self, line: u32, column: u32, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.active() else { return };
        tab.state.update(cx, |state, cx| {
            let last_line = state.text().lines_len().saturating_sub(1) as u32;
            state.set_cursor_position(Position::new(line.min(last_line), column), window, cx);
        });
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
        self.open_palette_with(mode, Vec::new(), Vec::new(), window, cx);
    }

    fn open_palette_with(
        &mut self,
        mode: PaletteMode,
        branches: Vec<vcs::Branch>,
        commits: Vec<vcs::Commit>,
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
            commits,
        };
        Palette::open(mode, source, window, cx);
    }

    fn close_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        // TODO: ask before discarding unsaved changes.
        self.tabs.remove(ix);
        self.active_tab = match self.active_tab {
            _ if self.tabs.is_empty() => None,
            Some(active) if active > ix => Some(active - 1),
            Some(active) => Some(active.min(self.tabs.len() - 1)),
            None => None,
        };
        match self.active_tab {
            Some(active) => self.activate_tab(active, window, cx),
            None => self.focus_handle.focus(window, cx),
        }
        cx.notify();
    }

    fn focus_editor(&self, window: &mut Window, cx: &mut Context<Self>) {
        match self.active() {
            Some(tab) => tab.state.update(cx, |state, cx| state.focus(window, cx)),
            None => self.focus_handle.focus(window, cx),
        }
    }

    fn terminal_focused(&self, window: &Window, cx: &App) -> bool {
        self.terminals
            .iter()
            .any(|t| t.read(cx).focus_handle().is_focused(window))
    }

    fn focus_terminal(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(terminal) = self.terminals.get(self.active_terminal) {
            terminal.read(cx).focus_handle().clone().focus(window, cx);
        }
    }

    fn new_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let session = match Session::spawn(self.root.clone()) {
            Ok(session) => session,
            Err(err) => return self.notify_error(format!("Cannot start terminal: {err:#}"), window, cx),
        };
        let terminal = cx.new(|cx| TerminalView::new(session, cx));
        let subscription = cx.subscribe_in(
            &terminal,
            window,
            |this, terminal, event, window, cx| match event {
                TerminalEvent::TitleChanged => cx.notify(),
                TerminalEvent::Exited => {
                    if let Some(ix) = this.terminals.iter().position(|t| t == terminal) {
                        this.close_terminal(ix, window, cx);
                    }
                }
            },
        );
        self._subscriptions.push(subscription);
        self.terminals.push(terminal);
        self.active_terminal = self.terminals.len() - 1;
        self.show_terminal = true;
        self.focus_terminal(window, cx);
        cx.notify();
    }

    fn close_terminal(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.terminals.len() {
            return;
        }
        let had_focus = self.terminal_focused(window, cx);
        self.terminals.remove(ix);
        if self.terminals.is_empty() {
            self.show_terminal = false;
            self.active_terminal = 0;
            self.focus_editor(window, cx);
        } else {
            self.active_terminal = self.active_terminal.min(self.terminals.len() - 1);
            if had_focus {
                self.focus_terminal(window, cx);
            }
        }
        cx.notify();
    }

    /// Alt+F12, JetBrains-style: open and focus, or hide when already focused.
    fn toggle_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.terminals.is_empty() {
            return self.new_terminal(window, cx);
        }
        if self.show_terminal && self.terminal_focused(window, cx) {
            self.show_terminal = false;
            self.focus_editor(window, cx);
        } else {
            self.show_terminal = true;
            self.focus_terminal(window, cx);
        }
        cx.notify();
    }

    fn save_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.active_tab else { return };
        let Some(CenterTab::Editor(tab)) = self.tabs.get_mut(ix) else {
            return;
        };
        let text = tab.state.read(cx).value();
        match fs::write(&tab.path, text.as_bytes()) {
            Ok(()) => {
                tab.saved_text = text;
                self.refresh_git(cx);
                cx.notify();
            }
            Err(err) => {
                let message = format!("Cannot save {}: {err}", tab.rel);
                self.notify_error(message, window, cx);
            }
        }
    }

    /// Applies a pure line operation to the active editor.
    fn edit_lines(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        op: impl FnOnce(&str, u32, &str) -> Option<LineEdit>,
    ) {
        let Some(tab) = self.active() else { return };
        let language = tab.language.clone();
        tab.state.update(cx, |state, cx| {
            let cursor = state.cursor_position();
            let text = state.value();
            if let Some(edit) = op(&text, cursor.line, &language) {
                state.replace_all(edit.text, window, cx);
                state.set_cursor_position(Position::new(edit.cursor_line, cursor.character), window, cx);
            }
        });
    }

    fn register_actions(&self, el: Div, cx: &mut Context<Self>) -> Div {
        el.on_action(cx.listener(|this, _: &SearchEverywhere, w, cx| {
            this.open_palette(PaletteMode::SearchEverywhere, w, cx)
        }))
        .on_action(cx.listener(|this, _: &GotoFile, w, cx| this.open_palette(PaletteMode::GotoFile, w, cx)))
        .on_action(
            cx.listener(|this, _: &RecentFiles, w, cx| this.open_palette(PaletteMode::RecentFiles, w, cx)),
        )
        .on_action(
            cx.listener(|this, _: &FindInFiles, w, cx| this.open_palette(PaletteMode::FindInFiles, w, cx)),
        )
        .on_action(
            cx.listener(|this, _: &FindAction, w, cx| this.open_palette(PaletteMode::FindAction, w, cx)),
        )
        .on_action(cx.listener(|this, _: &RecentProjects, w, cx| {
            this.open_palette(PaletteMode::RecentProjects, w, cx)
        }))
        .on_action(cx.listener(|this, _: &GotoLine, w, cx| {
            if this.active().is_some() {
                this.open_palette(PaletteMode::GotoLine, w, cx)
            }
        }))
        .on_action(cx.listener(|this, _: &SaveFile, w, cx| this.save_active(w, cx)))
        .on_action(cx.listener(|this, _: &CloseTab, w, cx| {
            if let Some(ix) = this.active_tab {
                this.close_tab(ix, w, cx)
            }
        }))
        .on_action(cx.listener(|this, _: &NextTab, w, cx| {
            if let Some(ix) = this.active_tab {
                this.activate_tab((ix + 1) % this.tabs.len(), w, cx)
            }
        }))
        .on_action(cx.listener(|this, _: &PrevTab, w, cx| {
            if let Some(ix) = this.active_tab {
                this.activate_tab(ix.checked_sub(1).unwrap_or(this.tabs.len() - 1), w, cx)
            }
        }))
        .on_action(
            cx.listener(|this, _: &ToggleProjectView, _, cx| this.toggle_left_panel(LeftPanel::Project, cx)),
        )
        .on_action(
            cx.listener(|this, _: &ToggleCommitView, _, cx| this.toggle_left_panel(LeftPanel::Commit, cx)),
        )
        .on_action(cx.listener(|this, _: &CommitChanges, w, cx| {
            this.show_project_view = true;
            this.left_panel = LeftPanel::Commit;
            this.focus_commit_message(w, cx);
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &Push, w, cx| this.push(w, cx)))
        .on_action(cx.listener(|this, _: &UpdateProject, w, cx| this.pull(w, cx)))
        .on_action(cx.listener(|this, _: &ShowGitLog, w, cx| this.open_git_log(w, cx)))
        .on_action(cx.listener(|this, _: &Branches, w, cx| this.open_branches(w, cx)))
        .on_action(cx.listener(|_, _: &CopyMcpCommand, w, cx| crate::mcp_bridge::copy_claude_command(w, cx)))
        .on_action(cx.listener(|_, _: &CloseWindow, w, _| w.remove_window()))
        .on_action(cx.listener(|this, _: &ToggleTerminal, w, cx| this.toggle_terminal(w, cx)))
        .on_action(cx.listener(|this, _: &NewTerminal, w, cx| this.new_terminal(w, cx)))
        .on_action(
            cx.listener(|this, _: &CloseTerminal, w, cx| this.close_terminal(this.active_terminal, w, cx)),
        )
        .on_action(cx.listener(|this, _: &ToggleLineComment, w, cx| {
            this.edit_lines(w, cx, |text, line, lang| {
                editor::toggle_line_comment(text, line, editor::line_comment_prefix(lang)?)
            })
        }))
        .on_action(cx.listener(|this, _: &DuplicateLine, w, cx| {
            this.edit_lines(w, cx, |text, line, _| editor::duplicate_line(text, line))
        }))
        .on_action(cx.listener(|this, _: &DeleteLine, w, cx| {
            this.edit_lines(w, cx, |text, line, _| editor::delete_line(text, line))
        }))
        .on_action(cx.listener(|this, _: &MoveLineUp, w, cx| {
            this.edit_lines(w, cx, |text, line, _| editor::move_line(text, line, true))
        }))
        .on_action(cx.listener(|this, _: &MoveLineDown, w, cx| {
            this.edit_lines(w, cx, |text, line, _| editor::move_line(text, line, false))
        }))
    }

    fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let file = self.active().map(|tab| tab.rel.clone());
        TitleBar::new().child(
            h_flex()
                .w_full()
                .justify_center()
                .gap_2()
                .text_sm()
                .child(style::signal_dot(theme.primary))
                .child(
                    div()
                        .text_color(theme.foreground)
                        .font_weight(FontWeight::MEDIUM)
                        .child(self.project_name()),
                )
                .children(file.map(|f| {
                    h_flex()
                        .gap_2()
                        .text_color(theme.muted_foreground)
                        .child(div().text_color(theme.border).child("/"))
                        .child(f)
                })),
        )
    }

    /// Cmd+1 / Cmd+0: show the panel, or hide the sidebar if it is already shown.
    fn toggle_left_panel(&mut self, panel: LeftPanel, cx: &mut Context<Self>) {
        if self.show_project_view && self.left_panel == panel {
            self.show_project_view = false;
        } else {
            self.show_project_view = true;
            self.left_panel = panel;
        }
        cx.notify();
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let changes = self.git.as_ref().map(|g| g.status.changes.len()).unwrap_or(0);
        let tab = |id: &'static str, label: String, panel: LeftPanel, cx: &mut Context<Self>| {
            let active = self.left_panel == panel;
            div()
                .id(id)
                .px_2()
                .py_1()
                .rounded_sm()
                .text_xs()
                .cursor_pointer()
                .text_color(if active {
                    cx.theme().foreground
                } else {
                    cx.theme().muted_foreground
                })
                .hover(|s| s.text_color(cx.theme().foreground))
                .child(label)
                .child(div().mt(px(3.)).h(px(2.)).rounded_full().when(active, |el| {
                    el.bg(cx.theme().primary)
                        .shadow(style::glow(cx.theme().primary, 0.7))
                }))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.left_panel = panel;
                    cx.notify();
                }))
        };
        let header = h_flex()
            .h(px(34.))
            .px_2()
            .gap_1()
            .border_b_1()
            .border_color(theme.border)
            .child(tab(
                "tab-project",
                self.project_name().to_uppercase(),
                LeftPanel::Project,
                cx,
            ))
            .child(tab(
                "tab-commit",
                if changes > 0 {
                    format!("COMMIT · {changes}")
                } else {
                    "COMMIT".into()
                },
                LeftPanel::Commit,
                cx,
            ));
        let body = match self.left_panel {
            LeftPanel::Project => self.render_project_view(cx).into_any_element(),
            LeftPanel::Commit => self.render_commit_panel(cx),
        };
        v_flex()
            .size_full()
            .bg(cx.theme().sidebar)
            .child(header)
            .child(div().flex_1().overflow_hidden().child(body))
    }

    fn render_project_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let workspace = cx.entity().downgrade();
        let active_rel = self.active().map(|tab| tab.rel.clone());

        v_flex()
            .size_full()
            .bg(theme.sidebar)
            .child(
                div()
                    .flex_1()
                    .child(tree(&self.tree, move |ix, entry, _selected, _, cx| {
                        let item = entry.item();
                        let id = item.id.to_string();
                        let is_folder = entry.is_folder();
                        let icon = match (is_folder, entry.is_expanded()) {
                            (true, true) => IconName::FolderOpen,
                            (true, false) => IconName::Folder,
                            _ => IconName::File,
                        };
                        let chevron = match (is_folder, entry.is_expanded()) {
                            (true, true) => Icon::new(IconName::ChevronDown)
                                .xsmall()
                                .text_color(cx.theme().muted_foreground)
                                .into_any_element(),
                            (true, false) => Icon::new(IconName::ChevronRight)
                                .xsmall()
                                .text_color(cx.theme().muted_foreground)
                                .into_any_element(),
                            _ => div().w(px(12.)).into_any_element(),
                        };
                        let is_active = active_rel.as_deref() == Some(id.as_str());
                        let label_color = workspace
                            .upgrade()
                            .and_then(|ws| ws.read(cx).git.as_ref().and_then(|g| g.kind_for(&id, is_folder)))
                            .map(|kind| git::kind_color(kind, cx))
                            .unwrap_or(cx.theme().foreground);
                        let workspace = workspace.clone();
                        ListItem::new(ix)
                            .selected(is_active)
                            .py(px(1.))
                            .pl(px(6.) + px(14.) * entry.depth() as f32)
                            .child(
                                h_flex()
                                    .gap_1p5()
                                    .text_sm()
                                    .child(chevron)
                                    .child(Icon::new(icon).small().text_color(cx.theme().muted_foreground))
                                    .child(div().text_color(label_color).child(item.label.clone())),
                            )
                            .on_click(move |_, window, cx| {
                                if !is_folder {
                                    workspace
                                        .update(cx, |ws, cx| ws.open_file(&id, None, window, cx))
                                        .ok();
                                }
                            })
                    })),
            )
    }

    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs = self.tabs.iter().enumerate().map(|(ix, tab)| {
            let dirty = tab.is_dirty(cx);
            Tab::new()
                .label(if dirty {
                    format!("● {}", tab.title())
                } else {
                    tab.title()
                })
                .suffix(
                    div()
                        .id(("close-tab", ix))
                        .px_1()
                        .rounded_sm()
                        .hover(|s| s.bg(cx.theme().muted))
                        .child(Icon::new(IconName::Close).xsmall())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.close_tab(ix, window, cx)
                        })),
                )
        });
        TabBar::new("editor-tabs")
            .underline()
            .children(tabs)
            .selected_index(self.active_tab.unwrap_or(0))
            .on_click(cx.listener(|this, ix: &usize, window, cx| this.activate_tab(*ix, window, cx)))
    }

    fn render_empty_state(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let hints: [(&str, Box<dyn Action>); 6] = [
            ("Search Everywhere", Box::new(SearchEverywhere)),
            ("Go to File", Box::new(GotoFile)),
            ("Recent Files", Box::new(RecentFiles)),
            ("Find in Files", Box::new(FindInFiles)),
            ("Find Action", Box::new(FindAction)),
            ("Open Project", Box::new(OpenProject)),
        ];
        let wordmark = v_flex()
            .items_center()
            .gap_3()
            .mb_6()
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(style::signal_dot(theme.primary))
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::LIGHT)
                            .text_color(theme.foreground)
                            .child("P O W E R   I D E A"),
                    ),
            )
            .child(style::fade_line(theme.primary, px(260.)));
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child(wordmark)
            .children(hints.into_iter().map(|(label, action)| {
                let keys = if action.partial_eq(&SearchEverywhere) {
                    div().child("Double ⇧").into_any_element()
                } else {
                    Kbd::binding_for_action(action.as_ref(), Some(WORKSPACE_CONTEXT), window)
                        .map(IntoElement::into_any_element)
                        .unwrap_or_else(|| div().into_any_element())
                };
                h_flex().w(px(300.)).justify_between().child(label).child(keys)
            }))
    }

    fn render_terminal_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let tabs = self.terminals.iter().enumerate().map(|(ix, terminal)| {
            let active = ix == self.active_terminal;
            h_flex()
                .id(("terminal-tab", ix))
                .h_full()
                .px_2()
                .gap_1()
                .text_xs()
                .border_b_2()
                .border_color(if active {
                    theme.primary
                } else {
                    gpui_kit::transparent_black()
                })
                .when(active, |el| el.shadow(style::glow(theme.primary, 0.4)))
                .text_color(if active {
                    theme.foreground
                } else {
                    theme.muted_foreground
                })
                .child(terminal.read(cx).title.clone())
                .child(
                    div()
                        .id(("close-terminal", ix))
                        .rounded_sm()
                        .hover(|s| s.bg(theme.muted))
                        .child(Icon::new(IconName::Close).xsmall())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.close_terminal(ix, window, cx)
                        })),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.active_terminal = ix;
                    this.focus_terminal(window, cx);
                    cx.notify();
                }))
        });

        v_flex()
            .size_full()
            .border_t_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .h(px(30.))
                    .px_2()
                    .gap_1()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .px_1()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child("Terminal"),
                    )
                    .children(tabs)
                    .child(
                        div()
                            .id("new-terminal")
                            .px_1()
                            .rounded_sm()
                            .hover(|s| s.bg(theme.muted))
                            .child(Icon::new(IconName::Plus).xsmall())
                            .on_click(cx.listener(|this, _, window, cx| this.new_terminal(window, cx))),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .id("hide-terminal")
                            .px_1()
                            .rounded_sm()
                            .hover(|s| s.bg(theme.muted))
                            .child(Icon::new(IconName::Minus).xsmall())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.show_terminal = false;
                                this.focus_editor(window, cx);
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .children(self.terminals.get(self.active_terminal).cloned()),
            )
    }

    fn render_status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mcp_port = cx
            .try_global::<crate::mcp_bridge::McpStatus>()
            .and_then(|s| s.port);
        let mcp = h_flex()
            .id("mcp-status")
            .gap_1p5()
            .px_1()
            .rounded_sm()
            .cursor_pointer()
            .hover(|s| s.bg(cx.theme().muted))
            .child(match mcp_port {
                Some(_) => style::signal_dot(cx.theme().green),
                None => div().size(px(6.)).rounded_full().bg(cx.theme().muted_foreground),
            })
            .child("MCP")
            .on_click(cx.listener(|_, _, window, cx| crate::mcp_bridge::copy_claude_command(window, cx)));
        let left =
            h_flex()
                .gap_3()
                .children(self.render_branch_label(cx))
                .child(match self.index.is_empty() {
                    true => "Indexing…".to_string(),
                    false => format!("{} files", self.index.len()),
                });
        let right = self.active().map(|tab| {
            let pos = tab.state.read(cx).cursor_position();
            h_flex()
                .gap_4()
                .child(format!("{}:{}", pos.line + 1, pos.character + 1))
                .child(tab.language.clone())
                .child("UTF-8")
        });
        StatusBar::new()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .left(left)
            .right(right.unwrap_or_else(h_flex).child(mcp))
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (background, foreground) = (cx.theme().background, cx.theme().foreground);
        let content = match self.active_tab.and_then(|ix| self.tabs.get(ix)) {
            Some(CenterTab::Editor(tab)) => Some(
                div()
                    .key_context(CODE_EDITOR_CONTEXT)
                    .size_full()
                    .child(Editor::new(&tab.state).bordered(false).size_full())
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
        let center = if self.show_terminal && !self.terminals.is_empty() {
            v_resizable("center")
                .child(resizable_panel().child(center))
                .child(
                    resizable_panel()
                        .size(px(280.))
                        .size_range(px(80.)..px(1600.))
                        .child(self.render_terminal_panel(cx)),
                )
                .into_any_element()
        } else {
            center
        };
        body = body.child(resizable_panel().child(center));

        let root = v_flex()
            .key_context(WORKSPACE_CONTEXT)
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(background)
            .text_color(foreground)
            .child(self.render_title_bar(cx))
            .child(div().flex_1().overflow_hidden().child(body))
            .child(self.render_status_bar(cx));
        self.register_actions(root, cx)
    }
}
