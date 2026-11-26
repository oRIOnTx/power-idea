//! Right sidebar: tool windows for things outside the project files — Database, Docker
//! and Remote Files — with their own activity bar at the window's right edge.

use gpui_kit::component::{ActiveTheme as _, Icon, Selectable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{Workspace, remote};
use crate::style;

pub(super) const DEFAULT_WIDTH: Pixels = px(300.);
const WIDTH_RANGE: (f32, f32) = (200., 700.);

/// Dragging the right sidebar's left edge.
#[derive(Clone)]
struct ResizeRight;

impl Render for ResizeRight {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// The new width when the left edge of a sidebar ending at `right` is dragged to `x`.
fn dragged_width(right: f32, x: f32) -> f32 {
    (right - x).clamp(WIDTH_RANGE.0, WIDTH_RANGE.1)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum RightPanel {
    Database,
    Docker,
    Remote,
}

impl RightPanel {
    pub(super) fn name(self) -> &'static str {
        match self {
            RightPanel::Database => "database",
            RightPanel::Docker => "docker",
            RightPanel::Remote => "remote",
        }
    }

    pub(super) fn from_name(name: &str) -> Option<Self> {
        match name {
            "database" => Some(RightPanel::Database),
            "docker" => Some(RightPanel::Docker),
            "remote" => Some(RightPanel::Remote),
            _ => None,
        }
    }

    fn title(self) -> &'static str {
        match self {
            RightPanel::Database => "Database",
            RightPanel::Docker => "Docker",
            RightPanel::Remote => "Remote Files",
        }
    }
}

impl Workspace {
    /// Shows `panel` on the right, or hides the right sidebar if `panel` is already shown.
    pub(super) fn toggle_right_panel(
        &mut self,
        panel: RightPanel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.right_panel == Some(panel) {
            self.right_panel = None;
        } else {
            self.show_right_panel(panel, window, cx);
        }
        cx.notify();
    }

    /// Shows `panel` on the right (starting what it needs on first use).
    pub(super) fn show_right_panel(
        &mut self,
        panel: RightPanel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.right_panel = Some(panel);
        match panel {
            RightPanel::Docker => self.ensure_docker(window, cx),
            RightPanel::Remote => self.ensure_remote(window, cx),
            RightPanel::Database => {}
        }
        cx.notify();
    }

    /// The vertical tool window stripe at the window's right edge.
    pub(super) fn render_right_activity_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let docker_running = self.docker.as_ref().map_or(0, |d| d.counts(&self.root).0);
        let remote_connected = self
            .remote
            .as_ref()
            .map_or(0, remote::RemoteState::connected_count);
        let accent = theme.primary;
        let slot = |id: &'static str,
                    icon: Icon,
                    tooltip: &'static str,
                    panel: RightPanel,
                    badge: usize,
                    cx: &mut Context<Self>| {
            let active = self.right_panel == Some(panel);
            div()
                .relative()
                .w_full()
                .flex()
                .justify_center()
                .py_1()
                // Accent rail on the inner (left) edge marks the open tool window.
                .child(
                    div()
                        .absolute()
                        .right_0()
                        .top(px(8.))
                        .bottom(px(8.))
                        .w(px(2.))
                        .rounded_full()
                        .when(active, |el| el.bg(accent).shadow(style::glow(accent, 0.8))),
                )
                .child(
                    style::icon_button_sized(id, icon, tooltip, style::BUTTON_ACTIVITY_BAR)
                        .selected(active)
                        .on_click(
                            cx.listener(move |ws, _, window, cx| ws.toggle_right_panel(panel, window, cx)),
                        ),
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
                            .bg(cx.theme().primary)
                            .text_color(cx.theme().primary_foreground)
                            .text_size(px(9.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(badge.to_string()),
                    )
                })
        };
        let (bg, border) = (theme.title_bar, theme.border);
        v_flex()
            .h_full()
            .w(px(52.))
            .flex_none()
            .py_1()
            .bg(bg)
            .border_l_1()
            .border_color(border)
            .child(slot(
                "rbar-database",
                Icon::empty().path("icons/database.svg"),
                "Database",
                RightPanel::Database,
                0,
                cx,
            ))
            .child(slot(
                "rbar-docker",
                style::app_icon("docker"),
                "Docker",
                RightPanel::Docker,
                docker_running,
                cx,
            ))
            .child(slot(
                "rbar-remote",
                style::lucide("server"),
                "Remote Files (⌘7)",
                RightPanel::Remote,
                remote_connected,
                cx,
            ))
    }

    /// The right sidebar as a column of its own beside the editor (like the left one),
    /// resizable by dragging its left edge.
    pub(super) fn render_right_column(&self, panel: RightPanel, cx: &mut Context<Self>) -> impl IntoElement {
        let (border, accent) = (cx.theme().border, cx.theme().primary);
        div()
            .id("right-sidebar")
            .relative()
            .flex_none()
            .h_full()
            .w(self.right_width)
            .border_l_1()
            .border_color(border)
            .on_drag_move(cx.listener(|ws, event: &DragMoveEvent<ResizeRight>, _, cx| {
                let width = dragged_width(f32::from(event.bounds.right()), f32::from(event.event.position.x));
                ws.right_width = px(width);
                cx.notify();
            }))
            .child(self.render_right_sidebar(panel, cx))
            .child(
                div()
                    .id("right-sidebar-handle")
                    .absolute()
                    .left(px(-3.))
                    .top_0()
                    .h_full()
                    .w(px(6.))
                    .cursor_col_resize()
                    .hover(move |s| s.bg(accent.opacity(0.3)))
                    .on_drag(ResizeRight, |_, _, _, cx| cx.new(|_| ResizeRight)),
            )
    }

    /// The open right tool window: header and body.
    pub(super) fn render_right_sidebar(&self, panel: RightPanel, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let header = h_flex()
            .h(px(34.))
            .px_3()
            .text_xs()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme.muted_foreground)
            .border_b_1()
            .border_color(theme.border)
            .child(panel.title().to_uppercase());
        let background = theme.sidebar;
        let body = match panel {
            RightPanel::Database => self.render_database_panel(cx),
            RightPanel::Docker => self.render_docker_panel(cx),
            RightPanel::Remote => self.render_remote_panel(cx),
        };
        v_flex()
            .size_full()
            .bg(background)
            .child(header)
            .child(div().flex_1().overflow_hidden().child(body))
    }
}

#[cfg(test)]
mod tests {
    use super::RightPanel;

    #[test]
    fn drag_width_is_clamped() {
        assert_eq!(super::dragged_width(1000., 700.), 300.);
        assert_eq!(super::dragged_width(1000., 950.), 200.);
        assert_eq!(super::dragged_width(1000., 100.), 700.);
    }

    #[test]
    fn names_round_trip() {
        for panel in [RightPanel::Database, RightPanel::Docker, RightPanel::Remote] {
            assert_eq!(RightPanel::from_name(panel.name()), Some(panel));
        }
        assert_eq!(RightPanel::from_name("project"), None);
    }
}
