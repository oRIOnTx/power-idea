//! Conflicts tab layout: operation bar (Continue / Abort), conflicted files on the
//! left, and the merge panel with Yours | Theirs per chunk on the right.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use vcs::{ConflictChunk, ConflictSegment, Operation, Resolution, Side};

use super::Workspace;
use crate::actions::GitLogOp;

/// Unchanged lines shown around each conflict.
const CONTEXT_LINES: usize = 3;

/// Lines of unchanged text to show: (head, number hidden, tail).
fn elide(text: &str, head: usize, tail: usize) -> (String, usize, String) {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= head + tail {
        return (lines.join("\n"), 0, String::new());
    }
    let (first, rest) = lines.split_at(head);
    let (hidden, last) = rest.split_at(rest.len() - tail);
    (first.join("\n"), hidden.len(), last.join("\n"))
}

/// Column titles for (ours, theirs): in a rebase "ours" is the branch being rebased onto.
fn side_titles(operation: Option<Operation>) -> (&'static str, &'static str) {
    match operation {
        Some(Operation::Rebase) => ("Upstream (ours)", "Your commit (theirs)"),
        _ => ("Yours", "Theirs"),
    }
}

impl Workspace {
    pub(super) fn render_conflicts_tab(&self, cx: &mut Context<Self>) -> AnyElement {
        let paths = self.conflicted_paths();
        let operation = self.git.as_ref().and_then(|g| g.operation);
        v_flex()
            .size_full()
            .child(self.render_operation_bar(operation, paths.len(), cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_start()
                    .child(self.render_conflict_files(&paths, cx))
                    .child(div().w(px(1.)).h_full().bg(cx.theme().border))
                    .child(self.render_merge_panel(operation, cx)),
            )
            .into_any_element()
    }

    fn render_operation_bar(
        &self,
        operation: Option<Operation>,
        conflicts: usize,
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = cx.theme();
        let text = match (operation, conflicts) {
            (Some(op), 0) => format!("{} in progress · all conflicts resolved", op.label()),
            (Some(op), n) => format!("{} in progress · {n} conflicted file{}", op.label(), plural(n)),
            (None, 0) => "No conflicts".to_string(),
            (None, n) => format!("{n} conflicted file{}", plural(n)),
        };
        let color = if conflicts > 0 { theme.warning } else { theme.green };
        h_flex()
            .px_3()
            .py_1p5()
            .gap_2()
            .text_sm()
            .border_b_1()
            .border_color(theme.border)
            .bg(color.opacity(0.10))
            .child(
                Icon::new(if conflicts > 0 {
                    IconName::TriangleAlert
                } else {
                    IconName::CircleCheck
                })
                .small()
                .text_color(color),
            )
            .child(div().flex_1().child(text))
            .when_some(operation, |bar, op| {
                bar.child(
                    Button::new("conflicts-continue")
                        .primary()
                        .xsmall()
                        .label(format!("Continue {}", op.label()))
                        .disabled(conflicts > 0)
                        .on_click(cx.listener(|ws, _, window, cx| {
                            ws.conflict_operation(GitLogOp::ContinueOperation, window, cx);
                        })),
                )
                .child(
                    Button::new("conflicts-abort")
                        .ghost()
                        .xsmall()
                        .label("Abort…")
                        .on_click(cx.listener(|ws, _, window, cx| {
                            ws.conflict_operation(GitLogOp::AbortOperation, window, cx);
                        })),
                )
            })
    }

    fn render_conflict_files(&self, paths: &[String], cx: &mut Context<Self>) -> Stateful<Div> {
        let theme = cx.theme();
        let selected = self.conflicts.selected.clone();
        v_flex()
            .id("conflict-files")
            .w(px(260.))
            .flex_none()
            .h_full()
            .overflow_y_scroll()
            .py_1()
            .children(paths.iter().enumerate().map(|(ix, path)| {
                let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
                let active = selected.as_deref() == Some(path.as_str());
                let path = path.clone();
                h_flex()
                    .id(("conflict-file", ix))
                    .mx_1()
                    .px_2()
                    .h(px(28.))
                    .gap_2()
                    .rounded(theme.radius)
                    .text_sm()
                    .cursor_pointer()
                    .when(active, |el| el.bg(theme.list_active))
                    .when(!active, |el| el.hover(|s| s.bg(theme.list_hover)))
                    .child(div().flex_none().text_color(theme.red).child(name.to_string()))
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(dir.to_string()),
                    )
                    .on_click(cx.listener(move |ws, _, _, cx| ws.select_conflict(path.clone(), cx)))
            }))
            .when(paths.is_empty(), |el| {
                el.child(
                    div()
                        .p_3()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child("Nothing to resolve"),
                )
            })
    }

    fn render_merge_panel(&self, operation: Option<Operation>, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme();
        let state = &self.conflicts;
        let Some(path) = state.selected.clone() else {
            return v_flex().flex_1();
        };
        let total = state.choices.len();
        let resolved = state.resolved_count();
        let titles = side_titles(operation);
        let progress = match (total, &state.error) {
            (_, Some(err)) => format!("Cannot show the text ({err}); choose a whole version"),
            (0, None) => "No conflict markers in the file; accept a version or mark it resolved".into(),
            (total, None) => format!("{resolved} of {total} conflict{} resolved", plural(total)),
        };
        let header = v_flex()
            .gap_1()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .gap_2()
                    .child(div().flex_1().min_w_0().text_sm().child(path))
                    .child(
                        Button::new("accept-ours")
                            .ghost()
                            .xsmall()
                            .label(format!("Accept {}", titles.0))
                            .on_click(cx.listener(|ws, _, window, cx| {
                                ws.accept_conflict_side(Side::Ours, window, cx);
                            })),
                    )
                    .child(
                        Button::new("accept-theirs")
                            .ghost()
                            .xsmall()
                            .label(format!("Accept {}", titles.1))
                            .on_click(cx.listener(|ws, _, window, cx| {
                                ws.accept_conflict_side(Side::Theirs, window, cx);
                            })),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(progress),
                    )
                    .child(
                        Button::new("open-in-editor")
                            .ghost()
                            .xsmall()
                            .label("Open in Editor")
                            .on_click(
                                cx.listener(|ws, _, window, cx| ws.open_conflict_in_editor(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("mark-resolved")
                            .ghost()
                            .xsmall()
                            .label("Mark as Resolved")
                            .tooltip("Stage the file after resolving it in the editor")
                            .on_click(cx.listener(|ws, _, window, cx| ws.mark_conflict_resolved(window, cx))),
                    )
                    .child(
                        Button::new("apply-merge")
                            .primary()
                            .xsmall()
                            .label("Apply")
                            .disabled(total == 0 || resolved < total)
                            .on_click(cx.listener(|ws, _, window, cx| ws.apply_merge(window, cx))),
                    ),
            );

        let last = state.segments.len().saturating_sub(1);
        let mut chunk = 0;
        let mut body = Vec::new();
        for (ix, segment) in state.segments.iter().enumerate() {
            match segment {
                ConflictSegment::Common(text) => {
                    let head = if ix == 0 { 0 } else { CONTEXT_LINES };
                    let tail = if ix == last { 0 } else { CONTEXT_LINES };
                    body.push(self.render_context(text, head, tail, cx));
                }
                ConflictSegment::Conflict(c) => {
                    body.push(self.render_chunk(
                        chunk,
                        c,
                        state.choices.get(chunk).copied().flatten(),
                        titles,
                        cx,
                    ));
                    chunk += 1;
                }
            }
        }
        v_flex().flex_1().min_w_0().h_full().child(header).child(
            v_flex()
                .id("merge-body")
                .flex_1()
                .overflow_y_scroll()
                .p_3()
                .gap_2()
                .children(body),
        )
    }

    fn render_context(&self, text: &str, head: usize, tail: usize, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let (top, hidden, bottom) = elide(text, head, tail);
        let code = |text: String| {
            div()
                .font_family(theme.mono_font_family.clone())
                .text_xs()
                .text_color(theme.muted_foreground)
                .whitespace_nowrap()
                .child(text)
        };
        v_flex()
            .px_2()
            .when(!top.is_empty(), |el| el.child(code(top)))
            .when(hidden > 0, |el| {
                el.child(
                    div()
                        .py_0p5()
                        .text_xs()
                        .text_color(theme.muted_foreground.opacity(0.7))
                        .child(format!("⋯ {hidden} unchanged line{}", plural(hidden))),
                )
            })
            .when(!bottom.is_empty(), |el| el.child(code(bottom)))
            .into_any_element()
    }

    fn render_chunk(
        &self,
        ix: usize,
        chunk: &ConflictChunk,
        choice: Option<Resolution>,
        titles: (&'static str, &'static str),
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let (ours_label, theirs_label) = chunk.labels();
        let takes = |side: Side| {
            matches!(
                (choice, side),
                (Some(Resolution::Ours), Side::Ours)
                    | (Some(Resolution::Theirs), Side::Theirs)
                    | (Some(Resolution::OursThenTheirs | Resolution::TheirsThenOurs), _)
            )
        };
        let column = |side: Side, title: &'static str, label: &str, text: &str, tint: Hsla| {
            let taken = takes(side);
            let dropped = choice.is_some() && !taken;
            let resolution = if side == Side::Ours {
                Resolution::Ours
            } else {
                Resolution::Theirs
            };
            v_flex()
                .flex_1()
                .min_w_0()
                .rounded(theme.radius)
                .border_1()
                .border_color(if taken { tint } else { theme.border })
                .bg(tint.opacity(if dropped { 0.03 } else { 0.10 }))
                .when(dropped, |el| el.opacity(0.5))
                .child(
                    h_flex()
                        .px_2()
                        .py_1()
                        .gap_2()
                        .text_xs()
                        .border_b_1()
                        .border_color(theme.border)
                        .child(div().text_color(tint).child(title))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .text_color(theme.muted_foreground)
                                .child(label.to_string()),
                        )
                        .child(
                            Button::new(("take", ix * 2 + usize::from(side == Side::Theirs)))
                                .xsmall()
                                .ghost()
                                .label(if choice == Some(resolution) {
                                    "Undo"
                                } else {
                                    "Accept"
                                })
                                .on_click(
                                    cx.listener(move |ws, _, _, cx| ws.choose_resolution(ix, resolution, cx)),
                                ),
                        ),
                )
                .child(
                    div()
                        .id(("chunk-text", ix * 2 + usize::from(side == Side::Theirs)))
                        .p_2()
                        .overflow_x_scroll()
                        .font_family(theme.mono_font_family.clone())
                        .text_xs()
                        .whitespace_nowrap()
                        .child(if text.is_empty() {
                            "(nothing)".to_string()
                        } else {
                            text.trim_end_matches('\n').to_string()
                        }),
                )
        };
        let both = |id: &'static str, label: &'static str, resolution: Resolution| {
            Button::new((id, ix))
                .xsmall()
                .ghost()
                .label(if choice == Some(resolution) { "Undo" } else { label })
                .on_click(cx.listener(move |ws, _, _, cx| ws.choose_resolution(ix, resolution, cx)))
        };
        v_flex()
            .gap_1()
            .child(
                h_flex()
                    .items_start()
                    .gap_2()
                    .child(column(
                        Side::Ours,
                        titles.0,
                        ours_label,
                        &chunk.ours,
                        theme.primary,
                    ))
                    .child(column(
                        Side::Theirs,
                        titles.1,
                        theirs_label,
                        &chunk.theirs,
                        theme.green,
                    )),
            )
            .child(
                h_flex()
                    .gap_1()
                    .text_xs()
                    .child(
                        div()
                            .flex_1()
                            .text_color(if choice.is_some() {
                                theme.green
                            } else {
                                theme.warning
                            })
                            .child(if choice.is_some() {
                                "✓ resolved"
                            } else {
                                "unresolved"
                            }),
                    )
                    .child(both(
                        "both-ours-first",
                        "Both, yours first",
                        Resolution::OursThenTheirs,
                    ))
                    .child(both(
                        "both-theirs-first",
                        "Both, theirs first",
                        Resolution::TheirsThenOurs,
                    )),
            )
            .into_any_element()
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

#[cfg(test)]
mod tests {
    use super::{elide, side_titles};
    use vcs::Operation;

    #[test]
    fn elides_long_context() {
        assert_eq!(elide("a\nb\n", 3, 3), ("a\nb".into(), 0, String::new()));
        let text = "1\n2\n3\n4\n5\n6\n7\n8\n";
        assert_eq!(elide(text, 2, 2), ("1\n2".into(), 4, "7\n8".into()));
        assert_eq!(elide(text, 0, 3), (String::new(), 5, "6\n7\n8".into()));
    }

    #[test]
    fn names_sides_by_operation() {
        assert_eq!(side_titles(Some(Operation::Merge)).0, "Yours");
        assert_eq!(side_titles(Some(Operation::Rebase)).1, "Your commit (theirs)");
    }
}
