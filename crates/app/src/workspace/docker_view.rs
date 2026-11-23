//! Docker panel rendering: header with compose actions, rows with live
//! status, ports and hover actions. State and actions live in `docker_panel`.

use std::process::Command;

use docker::{Action, Container, Group, GroupKind, Health, Row, State};
use gpui_kit::component::button::Button;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::Workspace;
use super::docker_panel::{DockerState, DockerStatus};
use crate::style;

impl Workspace {
    pub(super) fn render_docker_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let Some(state) = &self.docker else {
            return div().into_any_element();
        };
        let message = |text: String, retry: bool, cx: &mut Context<Self>| {
            let theme = cx.theme();
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .p_4()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(style::app_icon("docker").size_6())
                .child(div().text_center().child(text))
                .when(retry, |el| {
                    el.child(
                        Button::new("docker-retry")
                            .small()
                            .label("Check again")
                            .on_click(cx.listener(|ws, _, window, cx| ws.ensure_docker(window, cx))),
                    )
                })
                .into_any_element()
        };
        match &state.status {
            DockerStatus::Connecting => return message("Connecting to Docker…".into(), false, cx),
            DockerStatus::Unavailable(err) => return message(err.clone(), true, cx),
            DockerStatus::Ready => {}
        }
        let groups = state.groups(&self.root);
        let (running, total) = state.counts(&self.root);
        let title = self.docker_project_title();
        let has_compose = state.project.is_some();
        let show_all = state.show_all;

        let header = v_flex()
            .px_2()
            .py_1p5()
            .gap_1()
            .border_b_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(format!("{running}/{total} running")),
                    )
                    .when(has_compose, |el| {
                        el.child(
                            style::icon_button_sized(
                                "compose-up",
                                style::lucide("play"),
                                "docker compose up -d",
                                style::BUTTON_SMALL,
                            )
                            .on_click(cx.listener(|ws, _, window, cx| ws.compose("up -d", window, cx))),
                        )
                        .child(
                            style::icon_button_sized(
                                "compose-restart",
                                style::lucide("refresh-cw"),
                                "docker compose restart",
                                style::BUTTON_SMALL,
                            )
                            .on_click(cx.listener(|ws, _, window, cx| ws.compose("restart", window, cx))),
                        )
                        .child(
                            style::icon_button_sized(
                                "compose-down",
                                style::lucide("square"),
                                "docker compose down",
                                style::BUTTON_SMALL,
                            )
                            .on_click(cx.listener(|ws, _, window, cx| ws.compose("down", window, cx))),
                        )
                    }),
            )
            .child(
                Checkbox::new("docker-all")
                    .label("Show all containers")
                    .checked(show_all)
                    .on_click(cx.listener(|ws, checked: &bool, _, cx| {
                        if let Some(state) = ws.docker.as_mut() {
                            state.show_all = *checked;
                        }
                        cx.notify();
                    })),
            );

        let empty = groups.is_empty();
        let mut rows: Vec<AnyElement> = Vec::new();
        let mut ix = 0;
        for group in groups {
            let key = group_key(&group.kind);
            let collapsed = show_all && state.collapsed.contains(&key);
            // Without "Show all" there is only the project, so no header is needed.
            if show_all {
                rows.push(self.render_docker_group_header(&group, key, collapsed, cx));
            }
            if collapsed {
                ix += group.rows.len();
                continue;
            }
            for row in group.rows {
                rows.push(self.render_docker_row(ix, row, state, cx));
                ix += 1;
            }
        }
        v_flex()
            .size_full()
            .child(header)
            .child(
                div()
                    .id("docker-rows")
                    .flex_1()
                    .overflow_y_scroll()
                    .py_1()
                    .children(rows)
                    .when(empty, |el| {
                        el.child(
                            div()
                                .p_4()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child("No Docker configuration in this project. Tick \"Show all containers\" to see the rest."),
                        )
                    }),
            )
            .into_any_element()
    }

    /// The compose project's name, or the folder's without a compose file.
    fn docker_project_title(&self) -> String {
        self.docker
            .as_ref()
            .and_then(|d| d.project.as_ref())
            .map_or_else(|| self.project_name(), |p| p.name.clone())
    }

    /// Foldable group title with its running / total counts.
    fn render_docker_group_header(
        &self,
        group: &Group,
        key: String,
        collapsed: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let title = match &group.kind {
            GroupKind::Project => self.docker_project_title(),
            GroupKind::Compose(name) => name.clone(),
            GroupKind::Standalone => "Other containers".to_string(),
        };
        let running = group.rows.iter().filter(|r| r.state() == State::Running).count();
        let icon = match group.kind {
            GroupKind::Standalone => style::lucide("box"),
            _ => style::lucide("layers"),
        };
        h_flex()
            .id(SharedString::from(format!("docker-group-{key}")))
            .mx_1()
            .mt_1()
            .px_1()
            .py_0p5()
            .gap_1()
            .rounded(theme.radius)
            .text_xs()
            .cursor_pointer()
            .hover(|s| s.bg(theme.list_hover))
            .child(
                Icon::new(if collapsed {
                    IconName::ChevronRight
                } else {
                    IconName::ChevronDown
                })
                .xsmall()
                .text_color(theme.muted_foreground),
            )
            .child(Icon::new(icon).xsmall().text_color(theme.muted_foreground))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(
                div()
                    .text_color(theme.muted_foreground)
                    .child(format!("{running}/{}", group.rows.len())),
            )
            .on_click(cx.listener(move |ws, _, _, cx| {
                if let Some(state) = ws.docker.as_mut()
                    && !state.collapsed.remove(&key)
                {
                    state.collapsed.insert(key.clone());
                }
                cx.notify();
            }))
            .into_any_element()
    }

    fn render_docker_row(
        &self,
        ix: usize,
        row: Row,
        state: &DockerState,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let row_state = row.state();
        let exited_badly = row
            .container
            .as_ref()
            .is_some_and(|c| c.state == State::Exited && !c.status.starts_with("Exited (0)"));
        let color = match (row_state, row.container.as_ref().and_then(|c| c.health)) {
            (State::Running, Some(Health::Unhealthy)) => theme.danger,
            (State::Running, Some(Health::Starting)) | (State::Paused | State::Restarting, _) => theme.yellow,
            (State::Running, _) => theme.green,
            (State::Exited | State::Dead, _) if exited_badly => theme.danger,
            _ => theme.muted_foreground,
        };
        let busy = row.container.as_ref().is_some_and(|c| state.busy.contains(&c.id));
        let detail = row
            .container
            .as_ref()
            .map_or_else(|| "not created".to_string(), |c| c.status.clone());
        let image = row.container.as_ref().map_or_default(|c| c.image.clone());
        let ports = row.container.as_ref().map_or_default(Container::public_ports);

        let mut buttons = h_flex()
            .gap_0p5()
            .invisible()
            .group_hover("docker-row", Styled::visible);
        let button = |id: &'static str, icon: &str, tooltip: &'static str| {
            style::icon_button_sized((id, ix), style::lucide(icon), tooltip, style::BUTTON_SMALL)
        };
        match &row.container {
            None => {
                let service = row.title.clone();
                buttons = buttons.child(
                    button("docker-create", "play", "docker compose up -d <service>").on_click(cx.listener(
                        move |ws, _, window, cx| ws.compose(&format!("up -d {service}"), window, cx),
                    )),
                );
            }
            Some(container) => {
                let (id, name) = (container.id.clone(), container.name.clone());
                if container.state == State::Running {
                    let stop_id = id.clone();
                    buttons = buttons.child(button("docker-stop", "square", "Stop").on_click(cx.listener(
                        move |ws, _, window, cx| {
                            ws.container_action(stop_id.clone(), Action::Stop, window, cx);
                        },
                    )));
                } else {
                    let start_id = id.clone();
                    buttons = buttons.child(button("docker-start", "play", "Start").on_click(cx.listener(
                        move |ws, _, window, cx| {
                            ws.container_action(start_id.clone(), Action::Start, window, cx);
                        },
                    )));
                }
                let restart_id = id;
                let (logs_name, shell_name) = (name.clone(), name);
                let removed = container.clone();
                buttons = buttons
                    .child(button("docker-restart", "refresh-cw", "Restart").on_click(cx.listener(
                        move |ws, _, window, cx| ws.container_action(restart_id.clone(), Action::Restart, window, cx),
                    )))
                    .child(button("docker-logs", "scroll-text", "Logs").on_click(cx.listener(move |ws, _, window, cx| {
                        ws.docker_terminal(
                            format!("logs {logs_name}"),
                            format!("docker logs -f --tail 500 {logs_name}"),
                            window,
                            cx,
                        );
                    })))
                    .when(container.state == State::Running, |el| {
                        el.child(button("docker-shell", "square-terminal", "Shell").on_click(cx.listener(
                            move |ws, _, window, cx| {
                                ws.docker_terminal(
                                    format!("shell {shell_name}"),
                                    format!(
                                        "docker exec -it {shell_name} sh -c 'if command -v bash >/dev/null; then exec bash; else exec sh; fi'"
                                    ),
                                    window,
                                    cx,
                                );
                            },
                        )))
                    })
                    .child(button("docker-remove", "trash", "Remove…").on_click(cx.listener(
                        move |ws, _, window, cx| ws.confirm_remove(removed.clone(), window, cx),
                    )));
            }
        }

        let port_links = ports.into_iter().map(|port| {
            div()
                .id(SharedString::from(format!("docker-port-{ix}-{port}")))
                .flex_none()
                .px_1()
                .rounded_sm()
                .text_xs()
                .text_color(theme.primary)
                .bg(theme.primary.opacity(0.1))
                .cursor_pointer()
                .child(format!(":{port}"))
                .on_click(move |_, _, _| {
                    let _ = Command::new("open")
                        .arg(format!("http://localhost:{port}"))
                        .spawn();
                })
        });

        h_flex()
            .id(("docker-row", ix))
            .group("docker-row")
            .mx_1()
            .px_2()
            .py_1()
            .gap_2()
            .rounded(theme.radius)
            .hover(|s| s.bg(theme.list_hover))
            .child(div().flex_none().size(px(8.)).rounded_full().map(|el| {
                if row_state == State::NotCreated {
                    el.border_1().border_color(color)
                } else {
                    el.bg(color)
                }
            }))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        h_flex()
                            .gap_1p5()
                            .child(
                                div()
                                    .text_sm()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .whitespace_nowrap()
                                    .child(row.title),
                            )
                            .children(port_links),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .child(if image.is_empty() {
                                detail
                            } else {
                                format!("{image} · {detail}")
                            }),
                    ),
            )
            .child(if busy {
                div()
                    .text_xs()
                    .text_color(theme.primary)
                    .child("…")
                    .into_any_element()
            } else {
                buttons.into_any_element()
            })
            .into_any_element()
    }
}

fn group_key(kind: &GroupKind) -> String {
    match kind {
        GroupKind::Project => "project".into(),
        GroupKind::Compose(name) => format!("compose:{name}"),
        GroupKind::Standalone => "standalone".into(),
    }
}
