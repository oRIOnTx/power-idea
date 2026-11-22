//! Project tabs under the title bar. Project windows are grouped as native
//! macOS window tabs, but the custom title bar hides the native tab strip, so
//! we draw it ourselves.

use gpui_kit::component::{ActiveTheme as _, IconName, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{Workspace, Workspaces};
use crate::style;

/// Closes another project tab through its unsaved-changes guard.
fn close_project_window(handle: AnyWindowHandle, cx: &mut App) {
    let Some((_, workspace)) = Workspaces::live(cx).into_iter().find(|(h, _)| *h == handle) else {
        return;
    };
    handle
        .update(cx, |_, window, cx| {
            if workspace.update(cx, |ws, cx| ws.should_close_window(window, cx)) {
                window.remove_window();
            }
        })
        .ok();
}

impl Workspace {
    pub(super) fn render_window_tabs(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let tabs = window.tabbed_windows()?;
        // `tab_bar_visible` reports the native strip as shown, but the custom title bar
        // covers it, so draw ours whenever there is more than one project.
        if tabs.len() < 2 {
            return None;
        }
        let current = window.window_handle().window_id();
        let theme = cx.theme();
        let row = tabs.into_iter().enumerate().map(|(ix, tab)| {
            let active = tab.id == current;
            let (activate, close) = (tab.handle, tab.handle);
            h_flex()
                .id(("window-tab", ix))
                .group("window-tab")
                .relative()
                .flex_1()
                .min_w_0()
                .h_full()
                // Room on both sides keeps the title centred and the close button at the edge.
                .px(px(28.))
                .justify_center()
                .text_xs()
                .border_r_1()
                .border_color(theme.border)
                .cursor_pointer()
                .when(active, |el| el.bg(theme.background).text_color(theme.foreground))
                .when(!active, |el| {
                    el.text_color(theme.muted_foreground)
                        .hover(|s| s.bg(theme.list_hover))
                })
                // Accent line marks the active project.
                .when(active, |el| {
                    el.child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .right_0()
                            .h(px(2.))
                            .bg(theme.primary)
                            .shadow(style::glow(theme.primary, 0.6)),
                    )
                })
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(tab.title),
                )
                .child(
                    div()
                        .absolute()
                        .right(px(4.))
                        .top_0()
                        .bottom_0()
                        .flex()
                        .items_center()
                        .invisible()
                        .group_hover("window-tab", |s| s.visible())
                        .child(
                            style::icon_button(("close-window-tab", ix), IconName::Close, "Close Project")
                                .on_click(move |_, _, cx| {
                                    cx.stop_propagation();
                                    // The click may come from that very window, which is busy
                                    // handling the event; close it right after.
                                    cx.defer(move |cx| close_project_window(close, cx));
                                }),
                        ),
                )
                .on_click(move |_, _, cx| {
                    activate.update(cx, |_, window, _| window.activate_window()).ok();
                })
        });
        Some(
            h_flex()
                .h(px(30.))
                .flex_none()
                .bg(theme.title_bar)
                .border_b_1()
                .border_color(theme.border)
                .children(row)
                .into_any_element(),
        )
    }
}
