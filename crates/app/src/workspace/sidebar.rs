//! Left sidebar: Project tree (Cmd+1) and Commit tool window (Cmd+0).

use gpui_kit::component::list::ListItem;
use gpui_kit::component::tree::tree;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Selectable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{LeftPanel, Workspace, git};
use crate::style;

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

    /// Vertical tool window stripe at the far left, JetBrains new-UI style.
    pub(super) fn render_activity_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let changes = self.git.as_ref().map_or(0, |g| g.status.changes.len());
        let project_open = self.show_project_view && self.left_panel == LeftPanel::Project;
        let commit_open = self.show_project_view && self.left_panel == LeftPanel::Commit;
        let terminal_open = self.show_terminal;

        let slot = |id: &'static str,
                    icon: IconName,
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
                    style::icon_button(id, icon, tooltip)
                        .small()
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
            .w(px(44.))
            .flex_none()
            .py_1()
            .bg(theme.title_bar)
            .border_r_1()
            .border_color(theme.border)
            .child(slot(
                "bar-project",
                IconName::Folder,
                "Project (⌘1)",
                project_open,
                0,
                Box::new(cx.listener(|this, _, _, cx| this.toggle_left_panel(LeftPanel::Project, cx))),
            ))
            .child(slot(
                "bar-commit",
                IconName::CircleCheck,
                "Commit (⌘0)",
                commit_open,
                changes,
                Box::new(cx.listener(|this, _, _, cx| this.toggle_left_panel(LeftPanel::Commit, cx))),
            ))
            .child(div().flex_1())
            .child(slot(
                "bar-terminal",
                IconName::SquareTerminal,
                "Terminal (⌥F12)",
                terminal_open,
                0,
                Box::new(cx.listener(|this, _, window, cx| this.toggle_terminal(window, cx))),
            ))
            .child(slot(
                "bar-settings",
                IconName::Settings,
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

        v_flex().size_full().bg(theme.sidebar).child(
            div().flex_1().child(
                tree(&self.tree, move |ix, entry, _selected, _, cx| {
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
                        .map_or(cx.theme().foreground, |kind| git::kind_color(kind, cx));
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
                })
                .context_menu(move |_, entry, menu, _, cx| match menu_workspace.upgrade() {
                    Some(ws) => ws
                        .read(cx)
                        .file_context_menu(menu, &entry.item().id, entry.is_folder()),
                    None => menu,
                }),
            ),
        )
    }
}
