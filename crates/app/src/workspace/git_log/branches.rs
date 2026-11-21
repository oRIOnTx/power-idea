//! Branches panel of the Git Log: All / HEAD, Local and Remote trees.

use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenu};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use vcs::Branch;

use super::GitLogState;
use super::model::{BranchRow, branch_rows};
use super::view::ROW_HEIGHT;
use crate::actions::{GitLogAction, GitLogOp};
use crate::style;
use crate::workspace::Workspace;

fn branch_menu(menu: PopupMenu, branch: &Branch, focus: FocusHandle) -> PopupMenu {
    let action = |op| GitLogAction::boxed(op, branch.name.clone());
    let menu = menu.action_context(focus);
    let menu = if branch.is_current {
        menu
    } else {
        menu.menu("Checkout", action(GitLogOp::CheckoutBranch))
    };
    let menu = menu.menu("New Branch from Here…", action(GitLogOp::NewBranchAt));
    let menu = if branch.is_current {
        menu
    } else {
        menu.separator()
            .menu("Merge into Current", action(GitLogOp::MergeIntoCurrent))
            .menu("Rebase Current onto This", action(GitLogOp::RebaseCurrentOnto))
    };
    if branch.is_remote {
        return menu.separator().menu("Delete…", action(GitLogOp::DeleteBranch));
    }
    let menu = menu
        .separator()
        .menu("Push", action(GitLogOp::PushBranch))
        .menu("Rename…", action(GitLogOp::RenameBranch));
    if branch.is_current {
        menu
    } else {
        menu.menu("Delete…", action(GitLogOp::DeleteBranch))
    }
}

impl Workspace {
    pub(super) fn render_log_branches(&self, log: &GitLogState, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (list_active, list_hover, muted, primary, border) = (
            theme.list_active,
            theme.list_hover,
            theme.muted_foreground,
            theme.primary,
            theme.border,
        );
        let row = move |id: SharedString, depth: usize, selected: bool| {
            h_flex()
                .id(id)
                .h(ROW_HEIGHT)
                .pl(px(8. + 14. * depth as f32))
                .pr_2()
                .gap_1p5()
                .text_sm()
                .cursor_pointer()
                .when(selected, |el| el.bg(list_active))
                .when(!selected, |el| el.hover(move |s| s.bg(list_hover)))
        };
        let chevron = move |expanded: bool| {
            Icon::new(if expanded {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            })
            .with_size(style::TREE_CHEVRON)
            .text_color(muted)
        };

        let mut rows: Vec<AnyElement> = Vec::new();
        rows.push(
            row("log-all".into(), 0, log.branch.is_none())
                .child(style::lucide("git-graph").small().text_color(muted))
                .child("All branches")
                .on_click(cx.listener(|ws, _, _, cx| ws.set_log_branch(None, cx)))
                .into_any_element(),
        );
        if let Some(current) = log.branches.iter().find(|b| b.is_current) {
            let name = current.name.clone();
            rows.push(
                row("log-head".into(), 0, log.branch.as_deref() == Some("HEAD"))
                    .child(style::lucide("git-commit-vertical").small().text_color(primary))
                    .child("HEAD")
                    .child(div().text_xs().text_color(muted).child(name))
                    .on_click(cx.listener(|ws, _, _, cx| ws.set_log_branch(Some("HEAD".into()), cx)))
                    .into_any_element(),
            );
        }

        for item in branch_rows(&log.branches, &log.collapsed) {
            let element = match item {
                BranchRow::Section { key, title, expanded } => {
                    row(SharedString::from(format!("bs-{key}")), 0, false)
                        .mt_1()
                        .child(chevron(expanded))
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(muted)
                                .child(title),
                        )
                        .on_click(cx.listener(move |ws, _, _, cx| ws.toggle_branch_group(key.clone(), cx)))
                        .into_any_element()
                }
                BranchRow::Folder {
                    key,
                    name,
                    depth,
                    expanded,
                } => row(SharedString::from(format!("bf-{key}")), depth, false)
                    .child(chevron(expanded))
                    .child(Icon::new(IconName::Folder).small().text_color(muted))
                    .child(name)
                    .on_click(cx.listener(move |ws, _, _, cx| ws.toggle_branch_group(key.clone(), cx)))
                    .into_any_element(),
                BranchRow::Branch { ix, label, depth } => {
                    let branch = &log.branches[ix];
                    let selected = log.branch.as_deref() == Some(branch.name.as_str());
                    let base = row(SharedString::from(format!("bb-{}", branch.name)), depth, selected);
                    self.render_branch_row(base, branch, label, cx).into_any_element()
                }
            };
            rows.push(element);
        }

        div()
            .id("log-branches")
            .size_full()
            .overflow_y_scroll()
            .py_1()
            .border_r_1()
            .border_color(border)
            .children(rows)
    }

    fn render_branch_row(
        &self,
        row: Stateful<Div>,
        branch: &Branch,
        label: String,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let tint = if branch.is_current {
            theme.primary
        } else {
            theme.muted_foreground
        };
        let mut tracking = Vec::new();
        if branch.ahead > 0 {
            tracking.push((format!("↑{}", branch.ahead), theme.green));
        }
        if branch.behind > 0 {
            tracking.push((format!("↓{}", branch.behind), theme.yellow));
        }
        if branch.upstream_gone {
            tracking.push(("gone".to_string(), theme.danger));
        }
        let (name, checkout_name) = (branch.name.clone(), branch.name.clone());
        let menu_branch = branch.clone();
        let focus = self.focus_handle.clone();
        row.child(
            style::lucide("git-branch")
                .with_size(px(14.))
                .flex_none()
                .text_color(tint),
        )
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .when(branch.is_current, |el| el.font_weight(FontWeight::SEMIBOLD))
                .child(label),
        )
        .children(
            tracking
                .into_iter()
                .map(|(text, color)| div().flex_none().text_xs().text_color(color).child(text)),
        )
        .on_click(cx.listener(move |ws, event: &ClickEvent, window, cx| {
            if event.click_count() >= 2 {
                let action = GitLogAction {
                    op: GitLogOp::CheckoutBranch,
                    target: checkout_name.clone().into(),
                };
                ws.handle_git_log_action(&action, window, cx);
            } else {
                ws.set_log_branch(Some(name.clone()), cx);
            }
        }))
        .context_menu(move |menu, _, _| branch_menu(menu, &menu_branch, focus.clone()))
    }
}
