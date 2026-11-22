//! Window chrome: title bar widgets and the status bar.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, TitleBar, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::Workspace;
use crate::actions::*;
use crate::language::ServerStatus;
use crate::palette::PaletteMode;
use crate::style;

impl Workspace {
    pub(super) fn render_title_bar(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let run_widget = self.render_run_widget(cx).into_any_element();
        let theme = cx.theme();
        let file = self.active().map(|tab| tab.rel.clone());
        let branch = self.git.as_ref().map(|g| {
            let name = g.status.branch.clone().unwrap_or_else(|| "detached".into());
            (name, g.status.ahead, g.status.behind)
        });

        // JetBrains-style widget: label + chevron, opens a picker.
        let widget = |id: &'static str, label: String, tooltip: &'static str| {
            Button::new(id)
                .ghost()
                .xsmall()
                .label(label)
                .dropdown_caret(true)
                .tooltip(tooltip)
        };
        let focus = self.focus_handle.clone();
        let tool = |id: &'static str, icon: IconName, action: Box<dyn Action>, tooltip: &'static str| {
            let focus = focus.clone();
            style::icon_button_sized(id, icon, tooltip, style::BUTTON_TITLE_BAR).on_click(
                move |_, window, cx| {
                    // Actions route along the focus path, so make sure it runs through the workspace.
                    if !focus.contains_focused(window, cx) {
                        focus.focus(window, cx);
                    }
                    window.dispatch_action(action.boxed_clone(), cx);
                },
            )
        };

        // Traffic lights are hidden in full screen, so drop the space reserved for them.
        let title_bar = if window.is_fullscreen() {
            TitleBar::new().pl_3()
        } else {
            TitleBar::new()
        };
        title_bar.child(
            h_flex()
                .w_full()
                .pr_2()
                .gap_1()
                .text_sm()
                .child(style::signal_dot(theme.primary))
                .child(
                    widget("project-widget", self.project_name(), "Recent Projects").on_click(cx.listener(
                        |this, _, window, cx| this.open_palette(PaletteMode::RecentProjects, window, cx),
                    )),
                )
                .children(branch.map(|(branch, ahead, behind)| {
                    let tooltip = if ahead > 0 {
                        "Branches — commits to push"
                    } else {
                        "Branches"
                    };
                    widget("branch-widget", format!("⎇ {branch}"), tooltip)
                        .children(style::tracking_arrows(ahead, behind, cx))
                        .on_click(cx.listener(|this, _, window, cx| this.open_branches(window, cx)))
                }))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .text_center()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .children(file),
                )
                .child(run_widget)
                .child(div().w(px(1.)).h(px(16.)).mx_1().bg(theme.border))
                .child(tool(
                    "tb-search",
                    IconName::Search,
                    Box::new(SearchEverywhere),
                    "Search Everywhere (⇧⇧)",
                ))
                .child(div().w(px(1.)).h(px(16.)).mx_1().bg(theme.border))
                .child(tool(
                    "tb-open",
                    IconName::FolderOpen,
                    Box::new(OpenProject),
                    "Open Project…",
                )),
        )
    }

    pub(super) fn render_status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
        let servers = h_flex().gap_3().children(
            self.lsp
                .statuses()
                .into_iter()
                .map(|(id, status, fallback)| self.render_server_status(id, status, fallback, cx)),
        );
        StatusBar::new()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .left(left)
            .right(right.unwrap_or_else(h_flex).child(servers).child(mcp))
    }

    /// Status bar chip for one language server: starting / indexing / ready / failed.
    fn render_server_status(
        &self,
        id: String,
        status: ServerStatus,
        fallback: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let (dot, label) = match &status {
            ServerStatus::Starting => (
                div().size(px(6.)).rounded_full().bg(theme.yellow),
                format!("{id} · starting…"),
            ),
            ServerStatus::Working { title, percentage } => (
                style::signal_dot(theme.yellow),
                match percentage {
                    Some(p) => format!("{id} · {} {p}%", title.to_lowercase()),
                    None => format!("{id} · {}…", title.to_lowercase()),
                },
            ),
            ServerStatus::Ready if fallback => (style::signal_dot(theme.yellow), format!("{id} · fallback")),
            ServerStatus::Ready => (style::signal_dot(theme.primary), id.clone()),
            ServerStatus::Failed(_) => (
                div().size(px(6.)).rounded_full().bg(theme.danger),
                format!("{id} · failed"),
            ),
        };
        let failed = matches!(status, ServerStatus::Failed(_));
        h_flex()
            .id(SharedString::from(format!("lsp-status-{id}")))
            .gap_1p5()
            .px_1()
            .rounded_sm()
            .when(failed, |el| {
                el.text_color(theme.danger)
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.muted))
            })
            .child(dot)
            .child(label)
            .when(failed, |el| {
                el.tooltip(|window, cx| Tooltip::new("Click to restart").build(window, cx))
                    .on_click(cx.listener(move |ws, _, _, cx| ws.restart_language_server(&id, cx)))
            })
            .into_any_element()
    }
}
