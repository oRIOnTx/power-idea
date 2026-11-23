//! Left sidebar: Project tree (Cmd+1) and Commit tool window (Cmd+0), plus the activity bar.

use gpui_kit::component::input;
use gpui_kit::component::list::ListItem;
use gpui_kit::component::menu::ContextMenuExt as _;
use gpui_kit::component::tree::tree;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Selectable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{LeftPanel, Workspace, git};
use crate::actions::PROJECT_TREE_CONTEXT;
use crate::file_ops::ClipMode;
use crate::{file_icons, style};

/// Click callback for an activity bar slot.
type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

impl Workspace {
    /// Cmd+1 / Cmd+0: show the panel, or hide the sidebar if it is already shown.
    pub(super) fn toggle_left_panel(&mut self, panel: LeftPanel, cx: &mut Context<Self>) {
        if self.show_project_view && self.left_panel == panel {
            self.show_project_view = false;
        } else {
            self.show_project_view = true;
            self.left_panel = panel;
        }
        cx.notify();
    }

    /// Shows the Docker panel (connecting on first use) or hides the sidebar.
    pub(super) fn toggle_docker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_left_panel(LeftPanel::Docker, cx);
        if self.show_project_view && self.left_panel == LeftPanel::Docker {
            self.ensure_docker(window, cx);
        }
    }

    /// Vertical tool window stripe at the far left, JetBrains new-UI style.
    pub(super) fn render_activity_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let changes = self.git.as_ref().map_or(0, |g| g.status.changes.len());
        let project_open = self.show_project_view && self.left_panel == LeftPanel::Project;
        let commit_open = self.show_project_view && self.left_panel == LeftPanel::Commit;
        let database_open = self.show_project_view && self.left_panel == LeftPanel::Database;
        let docker_open = self.show_project_view && self.left_panel == LeftPanel::Docker;
        let docker_running = self.docker.as_ref().map_or(0, |d| d.counts(&self.root).0);
        let insights_open = self.show_project_view && self.left_panel == LeftPanel::Insights;
        let insights_enabled = Self::insights_enabled(cx);
        let terminal_open = self.terminal_visible();
        let log_open = self.git_log_visible();

        let slot = |id: &'static str,
                    icon: Icon,
                    tooltip: &'static str,
                    active: bool,
                    badge: usize,
                    on_click: ClickHandler| {
            let accent = theme.primary;
            div()
                .relative()
                .w_full()
                .flex()
                .justify_center()
                .py_1()
                // Accent rail marks the open tool window.
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .top(px(8.))
                        .bottom(px(8.))
                        .w(px(2.))
                        .rounded_full()
                        .when(active, |el| el.bg(accent).shadow(style::glow(accent, 0.8))),
                )
                .child(
                    style::icon_button_sized(id, icon, tooltip, style::BUTTON_ACTIVITY_BAR)
                        .selected(active)
                        .on_click(move |event, window, cx| on_click(event, window, cx)),
                )
                .when(badge > 0, |el| {
                    el.child(
                        div()
                            .absolute()
                            .top(px(2.))
                            .right(px(4.))
                            .min_w(px(14.))
                            .h(px(14.))
                            .px(px(3.))
                            .rounded_full()
                            .bg(theme.primary)
                            .text_color(theme.primary_foreground)
                            .text_size(px(9.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(if badge > 99 {
                                "99+".to_string()
                            } else {
                                badge.to_string()
                            }),
                    )
                })
        };

        v_flex()
            .h_full()
            .w(px(52.))
            .flex_none()
            .py_1()
            .bg(theme.title_bar)
            .border_r_1()
            .border_color(theme.border)
            .child(slot(
                "bar-project",
                Icon::new(IconName::Folder),
                "Project (⌘1)",
                project_open,
                0,
                Box::new(cx.listener(|this, _, _, cx| this.toggle_left_panel(LeftPanel::Project, cx))),
            ))
            .child(slot(
                "bar-commit",
                Icon::new(IconName::CircleCheck),
                "Commit (⌘0)",
                commit_open,
                changes,
                Box::new(cx.listener(|this, _, _, cx| this.toggle_left_panel(LeftPanel::Commit, cx))),
            ))
            .child(slot(
                "bar-database",
                Icon::empty().path("icons/database.svg"),
                "Database",
                database_open,
                0,
                Box::new(cx.listener(|this, _, _, cx| this.toggle_left_panel(LeftPanel::Database, cx))),
            ))
            .child(slot(
                "bar-docker",
                style::app_icon("docker"),
                "Docker",
                docker_open,
                docker_running,
                Box::new(cx.listener(|this, _, window, cx| this.toggle_docker(window, cx))),
            ))
            .when(insights_enabled, |bar| {
                bar.child(slot(
                    "bar-insights",
                    style::lucide("chart-line"),
                    "Claude Insights",
                    insights_open,
                    0,
                    Box::new(cx.listener(|this, _, window, cx| this.toggle_insights(window, cx))),
                ))
            })
            .child(div().flex_1())
            .child(slot(
                "bar-git-log",
                style::lucide("git-graph"),
                "Git Log (⌘9)",
                log_open,
                0,
                Box::new(cx.listener(|this, _, window, cx| this.toggle_git_log(window, cx))),
            ))
            .child(slot(
                "bar-terminal",
                Icon::new(IconName::SquareTerminal),
                "Terminal (⌥F12)",
                terminal_open,
                0,
                Box::new(cx.listener(|this, _, window, cx| this.toggle_terminal(window, cx))),
            ))
            .child(slot(
                "bar-settings",
                Icon::new(IconName::Settings),
                "Settings (⌘,)",
                false,
                0,
                Box::new(cx.listener(|this, _, window, cx| this.open_settings(window, cx))),
            ))
    }

    pub(super) fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let title = match self.left_panel {
            LeftPanel::Project => self.project_name(),
            LeftPanel::Commit => "Commit".to_string(),
            LeftPanel::Database => "Database".to_string(),
            LeftPanel::Docker => "Docker".to_string(),
            LeftPanel::Insights => "Claude Insights".to_string(),
        };
        let header = h_flex()
            .h(px(34.))
            .px_3()
            .text_xs()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme.muted_foreground)
            .border_b_1()
            .border_color(theme.border)
            .child(title.to_uppercase());
        let body = match self.left_panel {
            LeftPanel::Project => self.render_project_view(cx).into_any_element(),
            LeftPanel::Commit => self.render_commit_panel(cx),
            LeftPanel::Database => self.render_database_panel(cx),
            LeftPanel::Docker => self.render_docker_panel(cx),
            LeftPanel::Insights => self.render_insights_panel(cx),
        };
        v_flex()
            .size_full()
            .bg(cx.theme().sidebar)
            .child(header)
            .child(div().flex_1().overflow_hidden().child(body))
    }

    pub(super) fn render_project_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let menu_workspace = cx.entity().downgrade();
        let workspace = cx.entity().downgrade();
        let active_rel = self.active().map(|tab| tab.rel.clone());
        let selection = self.tree_selection.paths.clone();
        let root_collapsed = self.root_collapsed;
        let root_selected = self.root_row_selected();

        v_flex()
            .size_full()
            .bg(theme.sidebar)
            .key_context(PROJECT_TREE_CONTEXT)
            .track_focus(&self.project_focus)
            // Edit → Cut / Copy / Paste from the menu bar while the tree has focus.
            .on_action(cx.listener(|this, _: &input::Cut, w, cx| this.clip_selection(ClipMode::Cut, w, cx)))
            .on_action(cx.listener(|this, _: &input::Copy, w, cx| this.clip_selection(ClipMode::Copy, w, cx)))
            .on_action(cx.listener(|this, _: &input::Paste, w, cx| this.paste_into_selection(w, cx)))
            .child(self.render_project_root(cx))
            .when(!root_collapsed, |el| {
                el.child(
                    div().flex_1().child(
                        tree(&self.tree, move |ix, entry, _selected, _, cx| {
                            let item = entry.item();
                            let id = item.id.to_string();
                            let is_folder = entry.is_folder();
                            let icon = if is_folder {
                                let glyph = if entry.is_expanded() {
                                    IconName::FolderOpen
                                } else {
                                    IconName::Folder
                                };
                                Icon::new(glyph).text_color(file_icons::folder_tint(&item.label).color(cx))
                            } else {
                                file_icons::icon_for_file(&item.label, cx)
                            };
                            // Fixed-size slot so long names never squeeze the indentation.
                            let chevron_slot = div().flex_none().w(style::TREE_CHEVRON).flex().items_center();
                            let chevron = match (is_folder, entry.is_expanded()) {
                                (true, expanded) => chevron_slot.child(
                                    Icon::new(if expanded {
                                        IconName::ChevronDown
                                    } else {
                                        IconName::ChevronRight
                                    })
                                    .with_size(style::TREE_CHEVRON)
                                    .text_color(cx.theme().muted_foreground),
                                ),
                                (false, _) => chevron_slot,
                            };
                            let is_selected = if root_selected {
                                false
                            } else if selection.is_empty() {
                                active_rel.as_deref() == Some(id.as_str())
                            } else {
                                selection.contains(&id)
                            };
                            let label_color = workspace
                                .upgrade()
                                .and_then(|ws| {
                                    ws.read(cx).git.as_ref().and_then(|g| g.kind_for(&id, is_folder))
                                })
                                .map_or(cx.theme().foreground, |kind| git::kind_color(kind, cx));
                            let workspace = workspace.clone();
                            // The Tree overrides `selected` with its single cursor row, so the
                            // rest of a multi-selection is painted here.
                            let selected_bg = cx.theme().list_active;
                            ListItem::new(ix)
                                .selected(is_selected)
                                .when(is_selected, |item| item.bg(selected_bg))
                                .py(px(1.))
                                .pl(px(6.) + px(14.) * (entry.depth() + 1) as f32)
                                .child(
                                    h_flex()
                                        .w_full()
                                        .min_w_0()
                                        .gap_1p5()
                                        .text_sm()
                                        .child(chevron)
                                        .child(div().flex_none().child(icon.with_size(style::TREE_ICON)))
                                        .child(
                                            div()
                                                .min_w_0()
                                                .overflow_hidden()
                                                .text_ellipsis()
                                                .whitespace_nowrap()
                                                .text_color(label_color)
                                                .child(item.label.clone()),
                                        ),
                                )
                                .on_click(move |event, window, cx| {
                                    let modifiers = event.modifiers();
                                    workspace
                                        .update(cx, |ws, cx| {
                                            if ws.click_tree_entry(ix, &id, modifiers, cx) {
                                                cx.stop_propagation();
                                            } else if !is_folder {
                                                ws.open_file(&id, None, window, cx);
                                                // JetBrains: a single click previews the file and keeps
                                                // focus in the tree (⌘C / ⌘X / ⌦); a double click edits it.
                                                if event.click_count() < 2 {
                                                    ws.tree.update(cx, |tree, cx| tree.focus(window, cx));
                                                }
                                            }
                                        })
                                        .ok();
                                })
                        })
                        .context_menu(move |_, entry, menu, _, cx| {
                            match menu_workspace.upgrade() {
                                Some(ws) => ws.read(cx).file_context_menu(
                                    menu,
                                    &entry.item().id,
                                    entry.is_folder(),
                                    cx,
                                ),
                                None => menu,
                            }
                        }),
                    ),
                )
            })
    }

    /// JetBrains-style root row: the project folder's name and where it lives; folds the tree.
    fn render_project_root(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let home = dirs::home_dir().unwrap_or_default();
        let location = crate::welcome::paths::tilde(&self.root, &home);
        let expanded = !self.root_collapsed;
        let workspace = cx.entity().downgrade();
        h_flex()
            .id("project-root")
            .flex_none()
            .w_full()
            .min_w_0()
            .py(px(3.))
            .pl(px(6.))
            .pr_2()
            .gap_1p5()
            .text_sm()
            .cursor_pointer()
            .when(self.root_row_selected(), |row| row.bg(theme.list_active))
            .hover(|s| s.bg(theme.list_hover))
            .child(
                div()
                    .id("project-root-chevron")
                    .flex_none()
                    .w(style::TREE_CHEVRON)
                    .flex()
                    .items_center()
                    .on_click(cx.listener(|ws, _, _, cx| {
                        cx.stop_propagation();
                        ws.toggle_root_collapsed(cx);
                    }))
                    .child(
                        Icon::new(if expanded {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .with_size(style::TREE_CHEVRON)
                        .text_color(theme.muted_foreground),
                    ),
            )
            .child(
                Icon::new(if expanded {
                    IconName::FolderOpen
                } else {
                    IconName::Folder
                })
                .with_size(style::TREE_ICON)
                .flex_none()
                .text_color(theme.primary),
            )
            .child(
                div()
                    .flex_none()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.project_name()),
            )
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(location),
            )
            // Like any folder row: a click selects it, a double click or the chevron folds it.
            .on_click(cx.listener(|ws, event: &ClickEvent, _, cx| {
                if event.click_count() >= 2 {
                    ws.toggle_root_collapsed(cx);
                } else {
                    ws.select_root_row(cx);
                }
            }))
            .context_menu(move |menu, _, cx| match workspace.upgrade() {
                Some(ws) => ws.read(cx).root_context_menu(menu, cx),
                None => menu,
            })
    }

    fn toggle_root_collapsed(&mut self, cx: &mut Context<Self>) {
        self.root_collapsed = !self.root_collapsed;
        cx.notify();
    }
}
