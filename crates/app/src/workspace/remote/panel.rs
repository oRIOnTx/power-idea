//! Drawing the Remote Files panel: site picker and status, toolbar, path bar and
//! breadcrumbs, filter, the folder tree and the transfer queue.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use remote::Site;
use remote::listing::SortBy;

use super::drag::ProjectDrag;
use super::site_form::site_color;
use super::tree::{Row, visible_rows};
use super::{RemoteState, SiteConn, Status, Workspace};
use crate::style;

impl Workspace {
    pub(in crate::workspace) fn render_remote_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(state) = self.remote.as_ref() else {
            return div().into_any_element();
        };
        let background = cx.theme().sidebar;
        let body = match self.active_remote() {
            None => self.render_remote_empty(cx),
            Some((site, conn)) => match conn.map(|c| &c.status) {
                Some(Status::Connected | Status::Lost) => self.render_remote_browser(state, site, conn, cx),
                status => self.render_remote_offline(site, status, cx),
            },
        };
        v_flex()
            .size_full()
            .child(self.render_remote_toolbar(state, cx))
            .child(div().flex_1().min_h_0().child(body))
            .children(self.render_transfers(cx))
            .bg(background)
            .into_any_element()
    }

    fn render_remote_toolbar(&self, state: &RemoteState, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let workspace = cx.entity().downgrade();
        let active = self.active_remote();
        let label = active.map_or_else(|| "No site".to_string(), |(site, _)| site.name.clone());
        let sites: Vec<(String, String, String)> = state
            .sites
            .iter()
            .map(|s| (s.id.clone(), s.name.clone(), s.address()))
            .collect();
        let current = state.active.clone();
        let picker = Button::new("remote-site-picker")
            .ghost()
            .xsmall()
            .label(label)
            .dropdown_caret(true)
            .dropdown_menu(move |mut menu, _, _| {
                for (id, name, address) in &sites {
                    let workspace = workspace.clone();
                    let id = id.clone();
                    menu = menu.item(
                        PopupMenuItem::new(format!("{name}  —  {address}"))
                            .checked(current.as_deref() == Some(id.as_str()))
                            .on_click(move |_, window, cx| {
                                workspace
                                    .update(cx, |ws, cx| ws.select_remote_site(&id, window, cx))
                                    .ok();
                            }),
                    );
                }
                let (new_site, import) = (workspace.clone(), workspace.clone());
                menu.separator()
                    .item(PopupMenuItem::new("New Site…").on_click(move |_, window, cx| {
                        new_site.update(cx, |ws, cx| ws.new_remote_site(window, cx)).ok();
                    }))
                    .item(
                        PopupMenuItem::new("Import from ~/.ssh/config…").on_click(move |_, window, cx| {
                            import.update(cx, |ws, cx| ws.import_ssh_hosts(window, cx)).ok();
                        }),
                    )
            });
        let status_dot = active.map(|(site, conn)| {
            let (color, text) = match conn.map(|c| &c.status) {
                Some(Status::Connected) => (theme.green, "Connected".to_string()),
                Some(Status::Connecting) => (theme.yellow, "Connecting…".to_string()),
                Some(Status::Lost) => (
                    theme.yellow,
                    "Connection dropped; reconnects on the next action".to_string(),
                ),
                Some(Status::Failed(err)) => (theme.danger, format!("Error: {err}")),
                Some(Status::Idle) | None => (theme.muted_foreground, "Not connected".to_string()),
            };
            let tint = site_color(site.color.as_deref(), cx);
            h_flex()
                .gap_1()
                .children(tint.map(|c| div().w(px(3.)).h(px(14.)).rounded_full().bg(c)))
                .child(
                    div()
                        .id("remote-status")
                        .size(px(8.))
                        .rounded_full()
                        .bg(color)
                        .shadow(style::glow(color, 0.5))
                        .tooltip(move |window, cx| Tooltip::new(text.clone()).build(window, cx)),
                )
        });
        let connected = matches!(
            active.and_then(|(_, c)| c).map(|c| &c.status),
            Some(Status::Connected | Status::Lost)
        );
        let site_id = active.map(|(s, _)| s.id.clone());
        let sort = state.sort;
        let show_hidden = state.show_hidden;
        let workspace = cx.entity().downgrade();
        h_flex()
            .px_2()
            .py_1()
            .gap_1()
            .border_b_1()
            .border_color(theme.border)
            .children(status_dot)
            .child(div().flex_1().min_w_0().overflow_hidden().child(picker))
            .when_some(site_id.clone(), |bar, id| {
                let toggle_id = id.clone();
                bar.child(if connected {
                    style::icon_button_sized(
                        "remote-disconnect",
                        style::lucide("unplug"),
                        "Disconnect",
                        style::BUTTON_SMALL,
                    )
                    .on_click(cx.listener(move |ws, _, _, cx| ws.disconnect_remote_site(&toggle_id, cx)))
                } else {
                    style::icon_button_sized(
                        "remote-connect",
                        style::lucide("plug"),
                        "Connect",
                        style::BUTTON_SMALL,
                    )
                    .on_click(
                        cx.listener(move |ws, _, window, cx| ws.connect_remote_site(&toggle_id, window, cx)),
                    )
                })
                .child({
                    let id = id.clone();
                    style::icon_button_sized(
                        "remote-refresh",
                        style::lucide("refresh-cw"),
                        "Refresh",
                        style::BUTTON_SMALL,
                    )
                    .on_click(cx.listener(move |ws, _, window, cx| ws.refresh_remote(&id, window, cx)))
                })
                .child(
                    style::icon_button_sized(
                        "remote-hidden",
                        style::lucide(if show_hidden { "eye" } else { "eye-off" }),
                        if show_hidden {
                            "Hide Hidden Files"
                        } else {
                            "Show Hidden Files"
                        },
                        style::BUTTON_SMALL,
                    )
                    .on_click(cx.listener(|ws, _, _, cx| {
                        if let Some(state) = ws.remote.as_mut() {
                            state.show_hidden = !state.show_hidden;
                        }
                        cx.notify();
                    })),
                )
                .child(
                    Button::new("remote-sort")
                        .ghost()
                        .xsmall()
                        .icon(style::lucide("arrow-up-down"))
                        .tooltip("Sort")
                        .dropdown_menu(move |mut menu, _, _| {
                            for (by, label) in [
                                (SortBy::Name, "Name"),
                                (SortBy::Size, "Size"),
                                (SortBy::Modified, "Date Modified"),
                            ] {
                                let workspace = workspace.clone();
                                menu = menu.item(PopupMenuItem::new(label).checked(sort == by).on_click(
                                    move |_, _, cx| {
                                        workspace.update(cx, |ws, cx| ws.set_remote_sort(by, cx)).ok();
                                    },
                                ));
                            }
                            menu
                        }),
                )
                .child(
                    style::icon_button_sized(
                        "remote-edit-site",
                        IconName::Settings,
                        "Edit Site…",
                        style::BUTTON_SMALL,
                    )
                    .on_click(cx.listener(move |ws, _, window, cx| ws.edit_remote_site(&id, window, cx))),
                )
            })
            .child(
                style::icon_button("remote-new-site", IconName::Plus, "New Site…")
                    .on_click(cx.listener(|ws, _, window, cx| ws.new_remote_site(window, cx))),
            )
    }

    fn render_remote_empty(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        v_flex()
            .p_4()
            .gap_2()
            .items_center()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child(style::lucide("server").size_6())
            .child("No remote sites yet")
            .child(
                Button::new("remote-empty-new")
                    .small()
                    .label("Add Site")
                    .on_click(cx.listener(|ws, _, window, cx| ws.new_remote_site(window, cx))),
            )
            .child(
                Button::new("remote-empty-import")
                    .small()
                    .ghost()
                    .label("Import from ~/.ssh/config")
                    .on_click(cx.listener(|ws, _, window, cx| ws.import_ssh_hosts(window, cx))),
            )
            .into_any_element()
    }

    fn render_remote_offline(
        &self,
        site: &Site,
        status: Option<&Status>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let id = site.id.clone();
        let edit_id = site.id.clone();
        let (message, color) = match status {
            Some(Status::Connecting) => ("Connecting…".to_string(), theme.muted_foreground),
            Some(Status::Failed(err)) if !err.is_empty() => (err.clone(), theme.danger),
            _ => ("Not connected".to_string(), theme.muted_foreground),
        };
        let connecting = matches!(status, Some(Status::Connecting));
        v_flex()
            .p_4()
            .gap_2()
            .items_center()
            .text_sm()
            .child(div().font_weight(FontWeight::SEMIBOLD).child(site.name.clone()))
            .child(div().text_xs().text_color(theme.muted_foreground).child(format!(
                "{} · {}",
                site.protocol.label(),
                site.address()
            )))
            .child(div().text_xs().text_color(color).text_center().child(message))
            .when(!connecting, |el| {
                el.child(
                    Button::new("remote-offline-connect")
                        .small()
                        .primary()
                        .label("Connect")
                        .on_click(
                            cx.listener(move |ws, _, window, cx| ws.connect_remote_site(&id, window, cx)),
                        ),
                )
                .child(
                    Button::new("remote-offline-edit")
                        .small()
                        .ghost()
                        .label("Edit Site…")
                        .on_click(
                            cx.listener(move |ws, _, window, cx| ws.edit_remote_site(&edit_id, window, cx)),
                        ),
                )
            })
            .into_any_element()
    }

    fn render_remote_browser(
        &self,
        state: &RemoteState,
        site: &Site,
        conn: Option<&SiteConn>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(conn) = conn else {
            return div().into_any_element();
        };
        let (muted, border, hover) = {
            let theme = cx.theme();
            (theme.muted_foreground, theme.border, theme.list_hover)
        };
        let filter = state.filter.read(cx).value().to_string();
        let rows = visible_rows(&conn.dirs, &conn.expanded, &conn.root, state.show_hidden, &filter);
        let elements: Vec<AnyElement> = rows
            .iter()
            .enumerate()
            .map(|(ix, row)| match row {
                Row::Entry { entry, depth } => {
                    self.render_remote_row(ix, site, conn, entry, *depth, state, cx)
                }
                Row::Note { text, depth } => div()
                    .pl(px(12. + 16. * *depth as f32))
                    .py_0p5()
                    .text_xs()
                    .text_color(muted)
                    .child(text.clone())
                    .into_any_element(),
            })
            .collect();
        let root = conn.root.clone();
        let (site_id, drop_site, drop_root) = (site.id.clone(), site.id.clone(), root.clone());
        let (project_site, project_root) = (site.id.clone(), root.clone());
        let crumbs = remote::path::breadcrumbs(&root);
        let up_site = site.id.clone();
        let up = remote::path::parent(&root);
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .px_2()
                    .pt_1()
                    .gap_1()
                    .child(
                        style::icon_button_sized(
                            "remote-up",
                            style::lucide("folder-up"),
                            "Parent Folder",
                            style::BUTTON_SMALL,
                        )
                        .on_click(cx.listener(move |ws, _, window, cx| {
                            ws.set_remote_root(&up_site, &up, window, cx);
                        })),
                    )
                    .child(div().flex_1().child(Input::new(&state.path_input).xsmall())),
            )
            .child(
                h_flex()
                    .px_2()
                    .py_0p5()
                    .gap_0p5()
                    .flex_wrap()
                    .text_xs()
                    .text_color(muted)
                    .children(crumbs.into_iter().enumerate().map(|(ix, (name, path))| {
                        let site = site_id.clone();
                        div()
                            .id(("remote-crumb", ix))
                            .px_1()
                            .rounded_sm()
                            .cursor_pointer()
                            .hover(move |s| s.bg(hover))
                            .child(if ix == 0 { name } else { format!("{name} /") })
                            .on_click(cx.listener(move |ws, _, window, cx| {
                                ws.set_remote_root(&site, &path, window, cx);
                            }))
                    })),
            )
            .child(
                div()
                    .px_2()
                    .pb_1()
                    .child(Input::new(&state.filter).xsmall().cleanable(true)),
            )
            .child(
                div()
                    .id("remote-tree")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .border_t_1()
                    .border_color(border)
                    .py_1()
                    .children(elements)
                    // Dropping on empty space uploads into the folder shown at the top.
                    .on_drop(cx.listener(move |ws, paths: &ExternalPaths, window, cx| {
                        ws.drop_local_on_remote(&drop_site, &drop_root, paths.paths().to_vec(), window, cx);
                    }))
                    .on_drop(cx.listener(move |ws, drag: &ProjectDrag, window, cx| {
                        let local = ws.root.join(&drag.rel);
                        ws.drop_local_on_remote(&project_site, &project_root, vec![local], window, cx);
                    }))
                    .drag_over::<ExternalPaths>(|style, _, _, cx| style.bg(cx.theme().drop_target))
                    .drag_over::<ProjectDrag>(|style, _, _, cx| style.bg(cx.theme().drop_target)),
            )
            .into_any_element()
    }
}
