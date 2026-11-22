//! Debug tool window: toolbar (resume, step, stop), frames, variables tree
//! and the console with an expression evaluator.

use gpui_kit::component::input::Input;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, h_flex, h_resizable, resizable_panel,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{DebugSession, DebugStatus};
use crate::style;
use crate::workspace::Workspace;

const ROW: Pixels = px(22.);

impl Workspace {
    pub(in crate::workspace) fn render_debug_panel(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let session = self.debug.as_ref()?;
        let toolbar = self.render_debug_toolbar(session, cx).into_any_element();
        let frames = self.render_frames(session, cx);
        let variables = self.render_variables(session, cx);
        let console = Self::render_console(session, cx);
        let theme = cx.theme();
        Some(
            v_flex()
                .size_full()
                .border_t_1()
                .border_color(theme.border)
                .child(toolbar)
                .child(
                    div().flex_1().overflow_hidden().child(
                        h_resizable("debug-body")
                            .child(
                                resizable_panel()
                                    .size(px(260.))
                                    .size_range(px(160.)..px(600.))
                                    .child(frames),
                            )
                            .child(resizable_panel().child(variables))
                            .child(
                                resizable_panel()
                                    .size(px(420.))
                                    .size_range(px(200.)..px(900.))
                                    .child(console),
                            ),
                    ),
                )
                .into_any_element(),
        )
    }

    fn render_debug_toolbar(&self, session: &DebugSession, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let paused = matches!(session.status, DebugStatus::Paused { .. });
        let running = session.status == DebugStatus::Running;
        let active = session.is_active();
        let (status, color) = match &session.status {
            DebugStatus::Starting(text) => (text.clone(), theme.yellow),
            DebugStatus::Running => ("Running".to_string(), theme.green),
            DebugStatus::Paused { reason, .. } => (format!("Paused ({reason})"), theme.primary),
            DebugStatus::Ended(text) => (text.clone(), theme.muted_foreground),
        };
        let button =
            |id: &'static str, icon: &str, tooltip: &'static str, enabled: bool, command: &'static str| {
                style::icon_button_sized(id, style::lucide(icon), tooltip, style::BUTTON_SMALL)
                    .disabled(!enabled)
                    .on_click(cx.listener(move |ws, _, _, cx| match command {
                        "stop" => ws.stop_debug(cx),
                        command => ws.debug_step(command, cx),
                    }))
            };
        h_flex()
            .h(px(30.))
            .flex_none()
            .px_2()
            .gap_1()
            .border_b_1()
            .border_color(theme.border)
            .text_xs()
            .child(div().px_1().text_color(theme.muted_foreground).child("Debug"))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(session.name.clone()),
            )
            .child(div().px_1().text_color(color).child(status))
            .child(div().w(px(1.)).h(px(16.)).mx_1().bg(theme.border))
            .child(button("dbg-resume", "play", "Resume (⌥⌘R)", paused, "continue"))
            .child(button("dbg-pause", "pause", "Pause", running, "pause"))
            .child(button("dbg-over", "redo-dot", "Step Over (F8)", paused, "next"))
            .child(button(
                "dbg-into",
                "arrow-down-to-dot",
                "Step Into (F7)",
                paused,
                "stepIn",
            ))
            .child(button(
                "dbg-out",
                "arrow-up-from-dot",
                "Step Out (⇧F8)",
                paused,
                "stepOut",
            ))
            .child(button("dbg-stop", "square", "Stop (⌘F2)", active, "stop"))
            .child(div().flex_1())
            .child(
                style::icon_button("debug-hide", IconName::Minus, "Hide").on_click(cx.listener(
                    |ws, _, window, cx| {
                        ws.show_bottom = false;
                        ws.focus_editor(window, cx);
                        cx.notify();
                    },
                )),
            )
    }

    fn render_frames(&self, session: &DebugSession, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let rows: Vec<AnyElement> = session
            .frames
            .iter()
            .enumerate()
            .map(|(ix, frame)| {
                let selected = ix == session.selected_frame;
                let place = frame.path.as_ref().map(|p| {
                    let name = p.file_name().map_or_default(|n| n.to_string_lossy().into_owned());
                    format!("{name}:{}", frame.line)
                });
                h_flex()
                    .id(("debug-frame", ix))
                    .h(ROW)
                    .px_2()
                    .gap_2()
                    .text_sm()
                    .cursor_pointer()
                    .when(selected, |el| el.bg(theme.list_active))
                    .when(!selected, |el| el.hover(|s| s.bg(theme.list_hover)))
                    .when(frame.path.is_none(), |el| el.text_color(theme.muted_foreground))
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .child(frame.name.clone()),
                    )
                    .children(place.map(|p| {
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(p)
                    }))
                    .on_click(cx.listener(move |ws, _, window, cx| ws.select_frame(ix, window, cx)))
                    .into_any_element()
            })
            .collect();
        let empty = rows.is_empty();
        v_flex()
            .id("debug-frames")
            .size_full()
            .overflow_y_scroll()
            .border_r_1()
            .border_color(theme.border)
            .child(section_title("Frames", cx))
            .children(rows)
            .when(empty, |el| {
                el.child(hint("Frames appear when the program pauses", cx))
            })
            .into_any_element()
    }

    fn render_variables(&self, session: &DebugSession, cx: &mut Context<Self>) -> AnyElement {
        let mut rows = Vec::new();
        for scope in &session.scopes {
            self.variable_row(
                session,
                &scope.name,
                None,
                None,
                scope.reference,
                0,
                &mut rows,
                cx,
            );
        }
        let empty = rows.is_empty();
        v_flex()
            .id("debug-variables")
            .size_full()
            .overflow_y_scroll()
            .border_r_1()
            .border_color(cx.theme().border)
            .child(section_title("Variables", cx))
            .children(rows)
            .when(empty, |el| el.child(hint("No variables", cx)))
            .into_any_element()
    }

    /// One row, then its children when expanded (depth-first).
    #[allow(clippy::too_many_arguments)]
    fn variable_row(
        &self,
        session: &DebugSession,
        name: &str,
        value: Option<&str>,
        type_name: Option<&str>,
        reference: i64,
        depth: usize,
        rows: &mut Vec<AnyElement>,
        cx: &mut Context<Self>,
    ) {
        let theme = cx.theme();
        let expandable = reference != 0;
        let expanded = session.expanded.contains(&reference);
        let chevron = if expandable {
            Icon::new(if expanded {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            })
            .with_size(style::TREE_CHEVRON)
            .text_color(theme.muted_foreground)
            .into_any_element()
        } else {
            div().w(style::TREE_CHEVRON).into_any_element()
        };
        let row = h_flex()
            .id(SharedString::from(format!("var-{depth}-{reference}-{name}")))
            .h(ROW)
            .pl(px(6. + 14. * depth as f32))
            .pr_2()
            .gap_1p5()
            .text_sm()
            .when(expandable, |el| {
                el.cursor_pointer().hover(|s| s.bg(theme.list_hover))
            })
            .child(chevron)
            .child(div().flex_none().text_color(theme.blue).child(name.to_string()))
            .children(value.map(|v| {
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .font_family(theme.mono_font_family.clone())
                    .child(format!("= {v}"))
            }))
            .children(type_name.map(|t| {
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(t.to_string())
            }))
            .when(expandable, |el| {
                el.on_click(cx.listener(move |ws, _, _, cx| ws.toggle_variable(reference, cx)))
            });
        rows.push(row.into_any_element());
        if expandable && expanded {
            match session.children.get(&reference) {
                Some(children) => {
                    for child in children {
                        self.variable_row(
                            session,
                            &child.name,
                            Some(&child.value),
                            child.type_name.as_deref(),
                            child.reference,
                            depth + 1,
                            rows,
                            cx,
                        );
                    }
                }
                None => rows.push(hint("Loading…", cx)),
            }
        }
    }

    fn render_console(session: &DebugSession, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let lines: Vec<AnyElement> = session
            .console
            .iter()
            .map(|line| div().whitespace_nowrap().child(line.clone()).into_any_element())
            .collect();
        v_flex()
            .size_full()
            .child(section_title("Console", cx))
            .child(
                div()
                    .id("debug-console")
                    .flex_1()
                    .overflow_scroll()
                    .px_2()
                    .font_family(theme.mono_font_family.clone())
                    .text_xs()
                    .children(lines),
            )
            .child(
                div()
                    .p_1()
                    .border_t_1()
                    .border_color(theme.border)
                    .child(Input::new(&session.evaluate).xsmall()),
            )
            .into_any_element()
    }
}

fn section_title(title: &'static str, cx: &App) -> Div {
    div()
        .px_2()
        .py_1()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(cx.theme().muted_foreground)
        .child(title)
}

fn hint(text: &'static str, cx: &App) -> AnyElement {
    div()
        .px_3()
        .py_1()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(text)
        .into_any_element()
}
