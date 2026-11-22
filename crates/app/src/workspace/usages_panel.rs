//! Find Usages tool window (bottom panel): usages grouped by file, the match
//! highlighted in each line; it stays open while you jump between results.

use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{BottomPanel, Workspace};
use crate::actions::{OpenSelectedUsage, SelectNext, SelectPrev, USAGES_CONTEXT};
use crate::file_icons;
use crate::style;
use crate::usages::{UsageGroup, UsageRow, visible_rows};

const ROW_HEIGHT: Pixels = px(24.);

pub(super) struct UsagesState {
    /// What was searched, e.g. the symbol name.
    title: String,
    groups: Rc<Vec<UsageGroup>>,
    collapsed: HashSet<PathBuf>,
    /// (group, usage) of the highlighted result.
    selected: Option<(usize, usize)>,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
}

impl UsagesState {
    fn rows(&self) -> Vec<UsageRow> {
        visible_rows(&self.groups, &self.collapsed)
    }

    fn total(&self) -> usize {
        self.groups.iter().map(|g| g.usages.len()).sum()
    }
}

impl Workspace {
    pub(super) fn show_usages(
        &mut self,
        title: String,
        groups: Vec<UsageGroup>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let selected = (!groups.is_empty()).then_some((0, 0));
        self.usages = Some(UsagesState {
            title,
            groups: Rc::new(groups),
            collapsed: HashSet::new(),
            selected,
            scroll: UniformListScrollHandle::new(),
            focus,
        });
        self.show_bottom = true;
        self.bottom_panel = BottomPanel::Usages;
        cx.notify();
    }

    fn open_usage(&mut self, group: usize, usage: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.usages.as_mut() else { return };
        let Some(target) = state
            .groups
            .get(group)
            .and_then(|g| Some((g.path.clone(), g.usages.get(usage)?.clone())))
        else {
            return;
        };
        state.selected = Some((group, usage));
        self.jump_to(&target.0, target.1.line, target.1.column, window, cx);
    }

    /// Arrow keys move between usages, skipping file rows.
    fn step_usage(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(state) = self.usages.as_mut() else { return };
        let rows: Vec<(usize, usize)> = state
            .rows()
            .into_iter()
            .filter_map(|row| match row {
                UsageRow::Usage(g, u) => Some((g, u)),
                UsageRow::File(_) => None,
            })
            .collect();
        if rows.is_empty() {
            return;
        }
        let current = state.selected.and_then(|s| rows.iter().position(|r| *r == s));
        let next = match (current, forward) {
            (None, _) => 0,
            (Some(ix), true) => (ix + 1).min(rows.len() - 1),
            (Some(ix), false) => ix.saturating_sub(1),
        };
        state.selected = Some(rows[next]);
        if let Some(row_ix) = state
            .rows()
            .iter()
            .position(|r| *r == UsageRow::Usage(rows[next].0, rows[next].1))
        {
            state.scroll.scroll_to_item(row_ix, ScrollStrategy::Nearest);
        }
        cx.notify();
    }

    fn toggle_usage_file(&mut self, group: usize, cx: &mut Context<Self>) {
        let Some(state) = self.usages.as_mut() else { return };
        let Some(path) = state.groups.get(group).map(|g| g.path.clone()) else {
            return;
        };
        if !state.collapsed.remove(&path) {
            state.collapsed.insert(path);
        }
        cx.notify();
    }

    pub(super) fn render_usages(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let state = self.usages.as_ref()?;
        let theme = cx.theme();
        let files = state.groups.len();
        let total = state.total();
        let header = h_flex()
            .h(px(30.))
            .flex_none()
            .px_2()
            .gap_2()
            .border_b_1()
            .border_color(theme.border)
            .text_xs()
            .child(div().px_1().text_color(theme.muted_foreground).child("Find"))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(format!("Usages of {}", state.title)),
            )
            .child(div().text_color(theme.muted_foreground).child(format!(
                "{total} usage{} in {files} file{}",
                if total == 1 { "" } else { "s" },
                if files == 1 { "" } else { "s" }
            )))
            .child(div().flex_1())
            .child(
                style::icon_button("usages-hide", IconName::Minus, "Hide").on_click(cx.listener(
                    |ws, _, window, cx| {
                        ws.show_bottom = false;
                        ws.focus_editor(window, cx);
                        cx.notify();
                    },
                )),
            );

        let rows = Rc::new(state.rows());
        let groups = state.groups.clone();
        let selected = state.selected;
        let collapsed = Rc::new(state.collapsed.clone());
        let workspace = cx.entity().downgrade();
        let focus = state.focus.clone();
        let list = uniform_list("usage-rows", rows.len(), move |range, _, cx| {
            range
                .map(|ix| {
                    usage_row(
                        rows[ix], ix, &groups, &collapsed, selected, &workspace, &focus, cx,
                    )
                })
                .collect()
        })
        .track_scroll(&state.scroll)
        .size_full();

        Some(
            div()
                .key_context(USAGES_CONTEXT)
                .track_focus(&state.focus)
                .size_full()
                .flex()
                .flex_col()
                .border_t_1()
                .border_color(theme.border)
                .on_action(cx.listener(|ws, _: &SelectPrev, _, cx| ws.step_usage(false, cx)))
                .on_action(cx.listener(|ws, _: &SelectNext, _, cx| ws.step_usage(true, cx)))
                .on_action(cx.listener(|ws, _: &OpenSelectedUsage, window, cx| {
                    if let Some((g, u)) = ws.usages.as_ref().and_then(|s| s.selected) {
                        ws.open_usage(g, u, window, cx);
                    }
                }))
                .child(header)
                .child(div().flex_1().overflow_hidden().child(list))
                .into_any_element(),
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn usage_row(
    row: UsageRow,
    ix: usize,
    groups: &[UsageGroup],
    collapsed: &HashSet<PathBuf>,
    selected: Option<(usize, usize)>,
    workspace: &WeakEntity<Workspace>,
    focus: &FocusHandle,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let (workspace, focus) = (workspace.clone(), focus.clone());
    let base = h_flex()
        .id(("usage-row", ix))
        .h(ROW_HEIGHT)
        .pr_3()
        .gap_1p5()
        .text_sm()
        .cursor_pointer();
    match row {
        UsageRow::File(g) => {
            let group = &groups[g];
            let name = group
                .path
                .file_name()
                .map_or_default(|n| n.to_string_lossy().into_owned());
            base.pl_2()
                .hover(|s| s.bg(theme.list_hover))
                .child(
                    Icon::new(if collapsed.contains(&group.path) {
                        IconName::ChevronRight
                    } else {
                        IconName::ChevronDown
                    })
                    .with_size(style::TREE_CHEVRON)
                    .text_color(theme.muted_foreground),
                )
                .child(file_icons::icon_for_file(&name, cx).with_size(px(14.)))
                .child(div().flex_none().child(name))
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(group.label.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(group.usages.len().to_string()),
                )
                .on_click(move |_, _, cx| {
                    workspace.update(cx, |ws, cx| ws.toggle_usage_file(g, cx)).ok();
                })
                .into_any_element()
        }
        UsageRow::Usage(g, u) => {
            let usage = &groups[g].usages[u];
            let is_selected = selected == Some((g, u));
            base.pl(px(40.))
                .when(is_selected, |el| el.bg(theme.list_active))
                .when(!is_selected, |el| el.hover(|s| s.bg(theme.list_hover)))
                .child(
                    div()
                        .flex_none()
                        .w(px(44.))
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(format!("{}", usage.line + 1)),
                )
                .child(
                    h_flex()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .font_family(theme.mono_font_family.clone())
                        .child(div().flex_none().child(usage.before.clone()))
                        .child(
                            div()
                                .flex_none()
                                .rounded_sm()
                                .bg(theme.primary.opacity(0.22))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(usage.matched.clone()),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .child(usage.after.clone()),
                        ),
                )
                .on_click(move |_, window, cx| {
                    focus.focus(window, cx);
                    workspace
                        .update(cx, |ws, cx| ws.open_usage(g, u, window, cx))
                        .ok();
                })
                .into_any_element()
        }
    }
}
