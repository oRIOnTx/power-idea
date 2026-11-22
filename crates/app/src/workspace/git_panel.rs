//! Git UI: Commit tool window, branch widgets and the diff toolbar.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::diff::DiffMode;
use gpui_kit::component::input::Textarea;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use vcs::{ChangeKind, FileChange};

use super::Workspace;
use super::git::{GitState, kind_color, kind_letter};
use crate::editor::{CenterTab, DiffTab};
use crate::style;

impl Workspace {
    pub(super) fn render_branch_card(&self, git: &GitState, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let status = &git.status;
        let busy = git.busy.is_some();
        let badge = |text: String, color: Hsla| {
            div()
                .px_1p5()
                .rounded_sm()
                .text_xs()
                .bg(color.opacity(0.14))
                .text_color(color)
                .child(text)
        };
        let action = |id: &'static str, icon: IconName, label: &'static str, tooltip: &'static str| {
            Button::new(id)
                .ghost()
                .xsmall()
                .icon(icon)
                .label(label)
                .tooltip(tooltip)
                .disabled(busy)
        };

        v_flex()
            .m_2()
            .p_2()
            .gap_2()
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(theme.border)
            .bg(theme.background)
            .child(
                h_flex()
                    .gap_2()
                    .child(style::signal_dot(theme.primary))
                    .child(
                        div()
                            .id("branch-name")
                            .flex_1()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .cursor_pointer()
                            .hover(|s| s.text_color(theme.primary))
                            .child(status.branch.clone().unwrap_or_else(|| "detached HEAD".into()))
                            .on_click(cx.listener(|ws, _, window, cx| ws.open_branches(window, cx))),
                    )
                    .when(status.ahead > 0, |el| {
                        el.child(badge(format!("↑ {}", status.ahead), theme.green))
                    })
                    .when(status.behind > 0, |el| {
                        el.child(badge(format!("↓ {}", status.behind), theme.yellow))
                    }),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(match &status.upstream {
                        Some(upstream) => format!("tracking {upstream}"),
                        None => "no upstream · Push will publish this branch".to_string(),
                    }),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        action("pull", IconName::ArrowDown, "Pull", "Update Project (⌘T)")
                            .on_click(cx.listener(|ws, _, window, cx| ws.pull(window, cx))),
                    )
                    .child(
                        action("push", IconName::ArrowUp, "Push", "Push (⌘⇧K)")
                            .on_click(cx.listener(|ws, _, window, cx| ws.push(window, cx))),
                    )
                    .child(
                        action("fetch", IconName::RefreshCw, "Fetch", "Fetch all remotes")
                            .on_click(cx.listener(|ws, _, window, cx| ws.fetch(window, cx))),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("branches")
                            .ghost()
                            .xsmall()
                            .label("Branches")
                            .on_click(cx.listener(|ws, _, window, cx| ws.open_branches(window, cx))),
                    )
                    .child(
                        Button::new("log")
                            .ghost()
                            .xsmall()
                            .label("Log")
                            .tooltip("Git Log (⌘9)")
                            .on_click(cx.listener(|ws, _, window, cx| ws.show_git_log(window, cx))),
                    ),
            )
            .children(git.busy.map(|label| {
                h_flex()
                    .gap_2()
                    .text_xs()
                    .text_color(theme.primary)
                    .child(Icon::new(IconName::LoaderCircle).xsmall())
                    .child(format!("{label}…"))
            }))
    }

    pub(super) fn render_change_row(
        &self,
        git: &GitState,
        ix: usize,
        change: &FileChange,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let checked = git.checked.contains(&change.path);
        let viewing = self
            .active_tab
            .and_then(|i| self.tabs.get(i))
            .is_some_and(|t| matches!(t, CenterTab::Diff(d) if d.change_path.as_ref() == Some(&change.path)));
        let (dir, name) = change.path.rsplit_once('/').unwrap_or(("", &change.path));
        let color = kind_color(change.kind, cx);
        let path = change.path.clone();
        let (open_diff, open_file, rollback) = (change.clone(), change.path.clone(), change.clone());

        h_flex()
            .id(("change", ix))
            .group("change")
            .mx_1()
            // Nested under the group header: the checkbox lines up with the header's chevron.
            .pl_8()
            .pr_2()
            .h(px(28.))
            .gap_2()
            .rounded(theme.radius)
            .text_sm()
            .cursor_pointer()
            .when(viewing, |el| {
                el.bg(theme.list_active)
                    .border_1()
                    .border_color(theme.list_active_border)
            })
            .when(!viewing, |el| el.hover(|s| s.bg(theme.list_hover)))
            .child(
                Checkbox::new(("check", ix))
                    .checked(checked)
                    .on_click(cx.listener(move |ws, checked: &bool, _, cx| {
                        ws.set_checked(vec![path.clone()], *checked, cx);
                    })),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_2()
                    .child(
                        div()
                            .flex_none()
                            .text_color(color)
                            .when(change.kind == ChangeKind::Deleted, |el| el.line_through())
                            .child(name.to_string()),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(match &change.orig_path {
                                Some(orig) => format!("← {orig}"),
                                None => dir.to_string(),
                            }),
                    ),
            )
            .child(
                h_flex()
                    .invisible()
                    .group_hover("change", |s| s.visible())
                    .gap_0p5()
                    .child(
                        Button::new(("open", ix))
                            .ghost()
                            .xsmall()
                            .icon(IconName::FileText)
                            .tooltip("Open file")
                            .on_click(cx.listener(move |ws, _, window, cx| {
                                cx.stop_propagation();
                                ws.open_repo_file(&open_file, window, cx);
                            })),
                    )
                    .child(
                        Button::new(("rollback", ix))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Undo2)
                            .tooltip("Rollback…")
                            .on_click(cx.listener(move |ws, _, window, cx| {
                                cx.stop_propagation();
                                ws.rollback(rollback.clone(), window, cx);
                            })),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .w(px(18.))
                    .text_center()
                    .rounded_sm()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .bg(color.opacity(0.14))
                    .text_color(color)
                    .child(kind_letter(change.kind)),
            )
            .on_click(cx.listener(move |ws, event: &ClickEvent, window, cx| {
                if event.click_count() >= 2 {
                    ws.open_repo_file(&open_diff.path, window, cx);
                } else {
                    ws.open_change_diff(open_diff.clone(), window, cx);
                }
            }))
    }

    pub(super) fn render_change_group(
        &self,
        git: &GitState,
        title: &'static str,
        changes: Vec<(usize, &FileChange)>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if changes.is_empty() {
            return None;
        }
        let theme = cx.theme();
        let collapsed = git.collapsed.contains(title);
        let paths: Vec<String> = changes.iter().map(|(_, c)| c.path.clone()).collect();
        let checked = paths.iter().filter(|p| git.checked.contains(*p)).count();
        let all = checked == paths.len();

        let header = h_flex()
            .id(title)
            .mx_1()
            .px_2()
            .h(px(28.))
            .gap_2()
            .rounded(theme.radius)
            .cursor_pointer()
            .hover(|s| s.bg(theme.list_hover))
            .child(
                Checkbox::new(("group", title.len()))
                    .checked(all)
                    .indeterminate(checked > 0 && !all)
                    .on_click(
                        cx.listener(move |ws, value: &bool, _, cx| ws.set_checked(paths.clone(), *value, cx)),
                    ),
            )
            .child(
                Icon::new(if collapsed {
                    IconName::ChevronRight
                } else {
                    IconName::ChevronDown
                })
                .xsmall()
                .text_color(theme.muted_foreground),
            )
            .child(div().text_xs().font_weight(FontWeight::SEMIBOLD).child(title))
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(changes.len().to_string()),
            )
            .on_click(cx.listener(move |ws, _, _, cx| {
                if let Some(git) = ws.git.as_mut()
                    && !git.collapsed.remove(title)
                {
                    git.collapsed.insert(title);
                }
                cx.notify();
            }));

        let rows: Vec<AnyElement> = if collapsed {
            Vec::new()
        } else {
            changes
                .into_iter()
                .map(|(ix, change)| self.render_change_row(git, ix, change, cx).into_any_element())
                .collect()
        };
        Some(v_flex().pb_1().child(header).children(rows).into_any_element())
    }

    pub(super) fn render_commit_box(&self, git: &GitState, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let selected = git.checked.len();
        let busy = git.busy.is_some();
        let message = git.message.read(cx).value();
        let subject_len = message.lines().next().map_or(0, |l| l.chars().count());
        let has_message = !message.trim().is_empty();
        let can_commit = !busy && (git.amend || (selected > 0 && has_message));
        let hint = if git.amend {
            "Amend the last commit".to_string()
        } else if selected == 0 {
            "Select files to commit".to_string()
        } else if !has_message {
            "Write a commit message".to_string()
        } else {
            format!("{selected} file{} selected", if selected == 1 { "" } else { "s" })
        };
        let commit_label = match (git.amend, selected) {
            (true, _) => "Amend Commit".to_string(),
            (false, 0) => "Commit".to_string(),
            (false, n) => format!("Commit {n} file{}", if n == 1 { "" } else { "s" }),
        };

        v_flex()
            .p_2()
            .gap_2()
            .border_t_1()
            .border_color(theme.border)
            .bg(theme.background)
            .child(Textarea::new(&git.message))
            .child(
                h_flex()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(Checkbox::new("amend").label("Amend").checked(git.amend).on_click(
                        cx.listener(|ws, checked: &bool, window, cx| ws.set_amend(*checked, window, cx)),
                    ))
                    .child(div().flex_1())
                    .child(
                        div()
                            .when(subject_len > 72, |el| el.text_color(theme.warning))
                            .child(format!("{subject_len}/72")),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("commit")
                            .primary()
                            .small()
                            .icon(IconName::Check)
                            .label(commit_label)
                            .disabled(!can_commit)
                            .on_click(cx.listener(|ws, _, window, cx| ws.commit(false, window, cx))),
                    )
                    .child(
                        Button::new("commit-push")
                            .small()
                            .label("Commit & Push")
                            .disabled(!can_commit)
                            .on_click(cx.listener(|ws, _, window, cx| ws.commit(true, window, cx))),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format!("{hint} · ⌘↩ commits")),
            )
    }

    pub(super) fn render_commit_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(git) = &self.git else {
            let theme = cx.theme();
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(Icon::new(IconName::Inbox).text_color(theme.muted_foreground))
                .child("Not a git repository")
                .into_any_element();
        };

        let indexed = || git.status.changes.iter().enumerate();
        let conflicts: Vec<_> = indexed()
            .filter(|(_, c)| c.kind == ChangeKind::Conflicted)
            .collect();
        let tracked: Vec<_> = indexed()
            .filter(|(_, c)| !matches!(c.kind, ChangeKind::Conflicted | ChangeKind::Untracked))
            .collect();
        let untracked: Vec<_> = indexed()
            .filter(|(_, c)| c.kind == ChangeKind::Untracked)
            .collect();

        let groups: Vec<AnyElement> = [
            self.render_change_group(git, "Conflicts", conflicts, cx),
            self.render_change_group(git, "Changes", tracked, cx),
            self.render_change_group(git, "Unversioned Files", untracked, cx),
        ]
        .into_iter()
        .flatten()
        .collect();

        let theme = cx.theme();
        let clean = v_flex()
            .p_6()
            .items_center()
            .gap_2()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child(Icon::new(IconName::CircleCheck).text_color(theme.green))
            .child("Working tree clean")
            .when(git.status.ahead > 0, |el| {
                el.child(div().text_xs().child(format!(
                    "{} commit{} to push",
                    git.status.ahead,
                    if git.status.ahead == 1 { "" } else { "s" }
                )))
            });

        v_flex()
            .size_full()
            .child(self.render_branch_card(git, cx))
            .child(
                div()
                    .id("changes")
                    .flex_1()
                    .overflow_y_scroll()
                    .when(groups.is_empty(), |el| el.child(clean))
                    .children(groups),
            )
            .child(self.render_commit_box(git, cx))
            .into_any_element()
    }

    pub(super) fn render_diff_toolbar(&self, tab: &DiffTab, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let mode = tab.state.read(cx).mode();
        let position = tab.change_path.as_ref().and_then(|path| {
            let changes = &self.git.as_ref()?.status.changes;
            let ix = changes.iter().position(|c| &c.path == path)?;
            Some(format!("{} of {}", ix + 1, changes.len()))
        });
        let set_mode = |id: &'static str, label: &'static str, target: DiffMode, cx: &mut Context<Self>| {
            let state = tab.state.clone();
            Button::new(id)
                .xsmall()
                .label(label)
                .when(mode == target, |b| b.primary())
                .when(mode != target, |b| b.ghost())
                .on_click(cx.listener(move |_, _, _, cx| state.update(cx, |s, cx| s.set_mode(target, cx))))
        };

        h_flex()
            .h(px(34.))
            .px_3()
            .gap_2()
            .border_b_1()
            .border_color(theme.border)
            .bg(theme.tab_bar)
            .text_xs()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .text_color(theme.muted_foreground)
                    .child(tab.change_path.clone().unwrap_or_else(|| tab.title.clone())),
            )
            .children(tab.change_path.clone().map(|path| {
                let (prev, next, open) = (path.clone(), path.clone(), path);
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("prev-change")
                            .ghost()
                            .xsmall()
                            .icon(IconName::ChevronUp)
                            .tooltip("Previous changed file")
                            .on_click(cx.listener(move |ws, _, window, cx| {
                                ws.step_change(&prev, false, window, cx);
                            })),
                    )
                    .children(
                        position
                            .clone()
                            .map(|p| div().text_color(theme.muted_foreground).child(p)),
                    )
                    .child(
                        Button::new("next-change")
                            .ghost()
                            .xsmall()
                            .icon(IconName::ChevronDown)
                            .tooltip("Next changed file")
                            .on_click(
                                cx.listener(move |ws, _, window, cx| ws.step_change(&next, true, window, cx)),
                            ),
                    )
                    .child(
                        Button::new("open-changed-file")
                            .ghost()
                            .xsmall()
                            .icon(IconName::FileText)
                            .label("Open")
                            .on_click(
                                cx.listener(move |ws, _, window, cx| ws.open_repo_file(&open, window, cx)),
                            ),
                    )
            }))
            .child(
                h_flex()
                    .gap_0p5()
                    .child(set_mode("mode-split", "Side by side", DiffMode::Split, cx))
                    .child(set_mode("mode-unified", "Unified", DiffMode::Unified, cx)),
            )
    }
}
