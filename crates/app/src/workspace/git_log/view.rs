//! Git Log tool window layout: header, conflict banner and the commit list
//! with its graph.

use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenu};
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, h_resizable, resizable_panel, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use vcs::{Anchor, GraphRow, LogEntry, RefKind, RefLabel};

use super::GitLogState;
use super::model::format_date;
use crate::actions::{GIT_LOG_CONTEXT, GitLogAction, GitLogOp, SelectNext, SelectPrev};
use crate::style;
use crate::workspace::Workspace;

pub(super) const ROW_HEIGHT: Pixels = px(24.);
const LANE_WIDTH: f32 = 14.;
/// Wider graphs are clipped so the subject stays readable.
const MAX_LANES: usize = 12;

pub(super) fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn commit_menu(menu: PopupMenu, hash: &str, focus: FocusHandle) -> PopupMenu {
    let action = |op| GitLogAction::boxed(op, hash.to_string());
    menu.action_context(focus)
        .menu("Copy Revision Number", action(GitLogOp::CopyRevision))
        .menu("New Branch…", action(GitLogOp::NewBranchAt))
        .menu("New Tag…", action(GitLogOp::NewTagAt))
        .menu("Checkout Revision", action(GitLogOp::CheckoutRevision))
        .separator()
        .menu("Cherry-Pick", action(GitLogOp::CherryPick))
        .menu("Revert Commit", action(GitLogOp::Revert))
        .separator()
        .menu("Reset Current Branch to Here (Soft)", action(GitLogOp::ResetSoft))
        .menu(
            "Reset Current Branch to Here (Mixed)",
            action(GitLogOp::ResetMixed),
        )
        .menu(
            "Reset Current Branch to Here (Hard)…",
            action(GitLogOp::ResetHard),
        )
}

/// Draws one row of the commit graph: lines through the row and the commit node.
fn graph_cell(row: GraphRow, lanes: usize, colors: Rc<[Hsla]>) -> impl IntoElement {
    let paint = move |bounds: Bounds<Pixels>, (): (), window: &mut Window, _: &mut App| {
        let x = |lane: usize| bounds.left() + px(LANE_WIDTH * (lane as f32 + 0.5));
        let y = |anchor: Anchor| match anchor {
            Anchor::Top => bounds.top(),
            Anchor::Middle => bounds.center().y,
            Anchor::Bottom => bounds.bottom(),
        };
        let color = |lane: usize| {
            colors
                .get(lane % colors.len().max(1))
                .copied()
                .unwrap_or_default()
        };
        for segment in &row.segments {
            let from = point(x(segment.from.0), y(segment.from.1));
            let to = point(x(segment.to.0), y(segment.to.1));
            let mut path = PathBuilder::stroke(px(1.5));
            path.move_to(from);
            if from.x == to.x {
                path.line_to(to);
            } else if segment.from.1 == Anchor::Middle {
                // Leaving the node: bend sideways first, then run down into the lane.
                path.curve_to(to, point(to.x, from.y));
            } else {
                path.curve_to(to, point(from.x, to.y));
            }
            if let Ok(path) = path.build() {
                window.paint_path(path, color(segment.lane));
            }
        }
        let node = point(x(row.column), bounds.center().y);
        window.paint_quad(
            fill(Bounds::centered_at(node, size(px(8.), px(8.))), color(row.column)).corner_radii(px(4.)),
        );
    };
    div()
        .flex_none()
        .h_full()
        .w(px(LANE_WIDTH * lanes as f32))
        .overflow_hidden()
        .child(canvas(|_, _, _| (), paint).size_full())
}

fn ref_chip(label: &RefLabel, cx: &App) -> Div {
    let theme = cx.theme();
    let color = match label.kind {
        RefKind::Head => theme.primary,
        RefKind::Local => theme.green,
        RefKind::Remote => theme.magenta,
        RefKind::Tag => theme.yellow,
    };
    div()
        .flex_none()
        .px_1()
        .rounded_sm()
        .text_xs()
        .bg(color.opacity(0.14))
        .text_color(color)
        .child(label.name.clone())
}

/// Everything a commit row needs, cloned once per render of the list.
struct RowContext {
    entries: Rc<Vec<LogEntry>>,
    visible: Rc<Vec<usize>>,
    graph: Option<Rc<Vec<GraphRow>>>,
    lanes: usize,
    colors: Rc<[Hsla]>,
    selected: Option<String>,
    now: i64,
    workspace: WeakEntity<Workspace>,
    menu_focus: FocusHandle,
    list_focus: FocusHandle,
}

impl RowContext {
    fn row(&self, row_ix: usize, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let Some((entry_ix, entry)) = self
            .visible
            .get(row_ix)
            .and_then(|&ix| Some((ix, self.entries.get(ix)?)))
        else {
            return div().into_any_element();
        };
        let selected = self.selected.as_deref() == Some(entry.hash.as_str());
        let graph = self
            .graph
            .as_ref()
            .and_then(|graph| graph.get(entry_ix).cloned())
            .map(|row| graph_cell(row, self.lanes, self.colors.clone()));
        let (hash, menu_hash, right_hash) = (entry.hash.clone(), entry.hash.clone(), entry.hash.clone());
        let right_workspace = self.workspace.clone();
        let (workspace, menu_focus, list_focus) = (
            self.workspace.clone(),
            self.menu_focus.clone(),
            self.list_focus.clone(),
        );
        let muted = theme.muted_foreground;

        h_flex()
            .id(("log-row", row_ix))
            .h(ROW_HEIGHT)
            .pl_2()
            .pr_3()
            .gap_2()
            .text_sm()
            .cursor_pointer()
            .when(selected, |el| el.bg(theme.list_active))
            .when(!selected, |el| el.hover(|s| s.bg(theme.list_hover)))
            .children(graph)
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .children(entry.refs.iter().map(|r| ref_chip(r, cx)))
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .child(entry.subject.clone()),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .w(px(140.))
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .text_xs()
                    .text_color(muted)
                    .child(entry.author.clone()),
            )
            .child(
                div()
                    .flex_none()
                    .w(px(90.))
                    .text_xs()
                    .text_color(muted)
                    .child(format_date(entry.timestamp, self.now, &entry.local_date)),
            )
            // Right-click selects the commit the menu acts on, like JetBrains.
            .on_mouse_down(MouseButton::Right, move |_, _, cx| {
                right_workspace
                    .update(cx, |ws, cx| ws.select_commit(Some(right_hash.clone()), cx))
                    .ok();
            })
            .on_click(move |_, window, cx| {
                list_focus.focus(window, cx);
                workspace
                    .update(cx, |ws, cx| ws.select_commit(Some(hash.clone()), cx))
                    .ok();
            })
            .context_menu(move |menu, _, _| commit_menu(menu, &menu_hash, menu_focus.clone()))
            .into_any_element()
    }
}

impl Workspace {
    pub(in crate::workspace) fn render_git_log(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let log = self.git_log.as_ref()?;
        let body = h_resizable("git-log-body")
            .child(
                resizable_panel()
                    .size(px(220.))
                    .size_range(px(140.)..px(480.))
                    .child(self.render_log_branches(log, cx)),
            )
            .child(resizable_panel().child(self.render_log_list(log, cx)))
            .child(
                resizable_panel()
                    .size(px(320.))
                    .size_range(px(200.)..px(720.))
                    .child(self.render_commit_details(log, cx)),
            );
        let header = self.render_log_header(log, cx).into_any_element();
        let banner = self.render_operation_banner(log, cx);
        Some(
            v_flex()
                .size_full()
                .border_t_1()
                .border_color(cx.theme().border)
                .child(header)
                .children(banner)
                .child(div().flex_1().overflow_hidden().child(body))
                .into_any_element(),
        )
    }

    fn render_log_header(&self, log: &GitLogState, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let branch_chip = log.branch.clone().map(|branch| {
            h_flex()
                .gap_1()
                .pl_2()
                .rounded_sm()
                .text_xs()
                .bg(theme.primary.opacity(0.12))
                .text_color(theme.primary)
                .child(format!("Branch: {branch}"))
                .child(
                    style::icon_button("log-clear-branch", IconName::Close, "Show all branches")
                        .on_click(cx.listener(|ws, _, _, cx| ws.set_log_branch(None, cx))),
                )
        });
        h_flex()
            .h(px(30.))
            .px_2()
            .gap_2()
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .px_1()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("Git Log"),
            )
            .child(
                div()
                    .w(px(240.))
                    .child(Input::new(&log.filter).xsmall().cleanable(true)),
            )
            .children(branch_chip)
            .when(log.loading, |el| {
                el.child(
                    Icon::new(IconName::LoaderCircle)
                        .xsmall()
                        .text_color(theme.muted_foreground),
                )
            })
            .child(div().flex_1())
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format!("{} commits", log.visible.len())),
            )
            .child(
                style::icon_button("log-refresh", IconName::RefreshCw, "Refresh")
                    .on_click(cx.listener(|ws, _, _, cx| ws.reload_git_log(cx))),
            )
            .child(
                style::icon_button("log-hide", IconName::Minus, "Hide (⌘9)").on_click(cx.listener(
                    |ws, _, window, cx| {
                        ws.show_bottom = false;
                        ws.focus_editor(window, cx);
                        cx.notify();
                    },
                )),
            )
    }

    fn render_operation_banner(&self, log: &GitLogState, cx: &mut Context<Self>) -> Option<AnyElement> {
        let operation = log.operation?;
        let theme = cx.theme();
        let run = |op: GitLogOp| {
            cx.listener(move |ws: &mut Workspace, _: &ClickEvent, window, cx| {
                ws.handle_git_log_action(
                    &GitLogAction {
                        op,
                        target: "".into(),
                    },
                    window,
                    cx,
                );
            })
        };
        Some(
            h_flex()
                .px_3()
                .py_1()
                .gap_2()
                .text_sm()
                .bg(theme.warning.opacity(0.12))
                .border_b_1()
                .border_color(theme.border)
                .child(
                    Icon::new(IconName::TriangleAlert)
                        .small()
                        .text_color(theme.warning),
                )
                .child(div().flex_1().child(format!(
                    "{} in progress. Resolve conflicts in the Commit tool window (⌘0), then continue.",
                    operation.label()
                )))
                .child(
                    Button::new("op-continue")
                        .primary()
                        .xsmall()
                        .label("Continue")
                        .on_click(run(GitLogOp::ContinueOperation)),
                )
                .child(
                    Button::new("op-abort")
                        .ghost()
                        .xsmall()
                        .label("Abort…")
                        .on_click(run(GitLogOp::AbortOperation)),
                )
                .into_any_element(),
        )
    }

    fn render_log_list(&self, log: &GitLogState, cx: &mut Context<Self>) -> impl IntoElement {
        let graph = log.show_graph().then(|| log.graph.clone());
        let lanes = graph
            .as_ref()
            .map_or(0, |g| g.iter().map(|r| r.width).max().unwrap_or(1).min(MAX_LANES));
        let rows = RowContext {
            entries: log.entries.clone(),
            visible: log.visible.clone(),
            graph,
            lanes,
            colors: Self::graph_colors(cx),
            selected: log.selected.clone(),
            now: unix_now(),
            workspace: cx.entity().downgrade(),
            menu_focus: self.focus_handle.clone(),
            list_focus: log.focus.clone(),
        };
        let count = log.visible.len();
        let empty = count == 0 && !log.loading;
        let more = log.has_more().then(|| {
            let theme = cx.theme();
            h_flex()
                .flex_none()
                .h(ROW_HEIGHT)
                .px_3()
                .gap_2()
                .text_xs()
                .text_color(theme.muted_foreground)
                .border_t_1()
                .border_color(theme.border)
                .child(format!("Showing the latest {} commits", log.entries.len()))
                .child(
                    Button::new("log-load-more")
                        .ghost()
                        .xsmall()
                        .label("Load more")
                        .on_click(cx.listener(|ws, _, _, cx| ws.load_more_history(cx))),
                )
        });
        div()
            .key_context(GIT_LOG_CONTEXT)
            .track_focus(&log.focus)
            .size_full()
            .flex()
            .flex_col()
            .on_action(cx.listener(|ws, _: &SelectPrev, _, cx| ws.step_commit(-1, cx)))
            .on_action(cx.listener(|ws, _: &SelectNext, _, cx| ws.step_commit(1, cx)))
            .when(empty, |el| {
                el.flex()
                    .items_center()
                    .justify_center()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("No commits")
            })
            .when(!empty, |el| {
                el.child(
                    div().flex_1().overflow_hidden().child(
                        uniform_list("git-log-rows", count, move |range, _, cx| {
                            range.map(|ix| rows.row(ix, cx)).collect()
                        })
                        .track_scroll(&log.scroll)
                        .size_full(),
                    ),
                )
                .children(more)
            })
    }

    /// Lane colors of the commit graph.
    fn graph_colors(cx: &App) -> Rc<[Hsla]> {
        let t = cx.theme();
        Rc::new([t.primary, t.blue, t.green, t.magenta, t.yellow, t.cyan, t.red])
    }
}
