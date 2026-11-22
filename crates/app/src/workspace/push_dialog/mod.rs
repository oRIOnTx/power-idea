//! Push dialog (⌘⇧K), as in JetBrains: before anything leaves the machine it shows where
//! it goes (remote, URL, branch) and what goes there (commits and the files they touch).

mod tree;

use std::collections::{HashMap, HashSet};

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use vcs::{ChangeKind, CommitFile, Outgoing, PushTarget, Repo};

use super::Workspace;
use super::git::kind_color;
use crate::{file_icons, style};
use tree::TreeRow;

const BODY_HEIGHT: f32 = 400.;
const ROW_HEIGHT: f32 = 26.;
const INDENT: f32 = 16.;

enum Load {
    Loading,
    Failed(String),
    Ready(Outgoing),
}

pub struct PushDialog {
    workspace: WeakEntity<Workspace>,
    repo: Repo,
    load: Load,
    /// `None`: the branch row, i.e. all outgoing commits together.
    selected: Option<usize>,
    /// Files of single commits, by hash, loaded when a commit is first selected.
    commit_files: HashMap<String, Vec<CommitFile>>,
    collapsed: HashSet<String>,
    tags: bool,
    focus: FocusHandle,
    focused_once: bool,
    _tasks: Vec<Task<()>>,
}

impl PushDialog {
    fn new(workspace: WeakEntity<Workspace>, repo: Repo, branch: String, cx: &mut Context<Self>) -> Self {
        let load_repo = repo.clone();
        let load = cx.spawn(async move |this, cx| {
            let outgoing = cx
                .background_spawn(async move { load_repo.outgoing(&branch) })
                .await;
            this.update(cx, |this, cx| {
                this.load = match outgoing {
                    Ok(outgoing) => Load::Ready(outgoing),
                    Err(err) => Load::Failed(format!("{err:#}")),
                };
                cx.notify();
            })
            .ok();
        });
        Self {
            workspace,
            repo,
            load: Load::Loading,
            selected: None,
            commit_files: HashMap::new(),
            collapsed: HashSet::new(),
            tags: false,
            focus: cx.focus_handle(),
            focused_once: false,
            _tasks: vec![load],
        }
    }

    fn outgoing(&self) -> Option<&Outgoing> {
        match &self.load {
            Load::Ready(outgoing) => Some(outgoing),
            _ => None,
        }
    }

    /// What the Push button would push to, if pushing makes sense.
    fn pushable(&self) -> Option<(&Outgoing, &PushTarget)> {
        let outgoing = self.outgoing()?;
        let target = outgoing.target.as_ref()?;
        let anything = !outgoing.commits.is_empty() || target.new_branch || self.tags;
        anything.then_some((outgoing, target))
    }

    fn select(&mut self, selected: Option<usize>, cx: &mut Context<Self>) {
        self.selected = selected;
        cx.notify();
        let Some(hash) = selected
            .and_then(|ix| self.outgoing()?.commits.get(ix))
            .map(|c| c.hash.clone())
            .filter(|hash| !self.commit_files.contains_key(hash))
        else {
            return;
        };
        let repo = self.repo.clone();
        self._tasks.push(cx.spawn(async move |this, cx| {
            let files = cx
                .background_spawn({
                    let hash = hash.clone();
                    async move { repo.commit_details(&hash) }
                })
                .await
                .map_or_default(|details| details.files);
            this.update(cx, |this, cx| {
                this.commit_files.insert(hash, files);
                cx.notify();
            })
            .ok();
        }));
    }

    /// Files of the selection; `None` while a commit's files are loading.
    fn files(&self) -> Option<&[CommitFile]> {
        let outgoing = self.outgoing()?;
        match self.selected {
            None => Some(&outgoing.files),
            Some(ix) => self
                .commit_files
                .get(&outgoing.commits.get(ix)?.hash)
                .map(Vec::as_slice),
        }
    }

    fn step(&mut self, forward: bool, cx: &mut Context<Self>) {
        let count = self.outgoing().map_or(0, |o| o.commits.len());
        let next = match (self.selected, forward) {
            (None, true) if count > 0 => Some(0),
            (Some(ix), true) if ix + 1 < count => Some(ix + 1),
            (Some(0), false) => None,
            (Some(ix), false) => Some(ix - 1),
            (current, _) => current,
        };
        self.select(next, cx);
    }

    fn push(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((outgoing, target)) = self.pushable() else {
            return;
        };
        let (local, target, tags) = (outgoing.local.clone(), target.clone(), self.tags);
        window.close_dialog(cx);
        self.workspace
            .update(cx, |ws, cx| ws.push_to(local, target, tags, window, cx))
            .ok();
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let modifiers = event.keystroke.modifiers;
        if modifiers.control || modifiers.alt || modifiers.shift {
            return;
        }
        match event.keystroke.key.as_str() {
            "up" if !modifiers.platform => self.step(false, cx),
            "down" if !modifiers.platform => self.step(true, cx),
            "enter" => self.push(window, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    fn render_summary(&self, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let line = h_flex().gap_2().text_sm().min_w_0();
        let Some(outgoing) = self.outgoing() else {
            return line.into_any_element();
        };
        let Some(target) = &outgoing.target else {
            return line
                .text_color(theme.warning)
                .child("This repository has no remote to push to.")
                .into_any_element();
        };
        let what = match outgoing.commits.len() {
            0 if target.new_branch => "No new commits; publishes the branch".to_string(),
            0 => "Nothing to push: the branch is up to date".to_string(),
            1 => "1 commit".to_string(),
            n => format!("{n} commits"),
        };
        v_flex()
            .gap_1()
            .child(
                line.child(div().font_weight(FontWeight::SEMIBOLD).child(what))
                    .child(div().text_color(muted).child("to"))
                    .child(style::lucide("server").small().text_color(muted))
                    .child(target.remote.clone())
                    .children(target.url.clone().map(|url| {
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .text_xs()
                            .text_color(muted)
                            .child(url)
                    })),
            )
            .when(outgoing.behind > 0, |el| {
                el.child(
                    h_flex()
                        .gap_2()
                        .text_xs()
                        .text_color(theme.warning)
                        .child(Icon::new(IconName::TriangleAlert).xsmall())
                        .child(format!(
                            "{} has {} commit{} that {} lacks: the push will be rejected. Update the project first.",
                            target.name(),
                            outgoing.behind,
                            if outgoing.behind == 1 { "" } else { "s" },
                            outgoing.local,
                        )),
                )
            })
            .into_any_element()
    }

    fn render_commits(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let list = v_flex()
            .id("push-commits")
            .flex_1()
            .min_w_0()
            .h_full()
            .py_1()
            .overflow_y_scroll();
        let outgoing = match &self.load {
            Load::Loading => {
                return list
                    .p_3()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child("Loading…")
                    .into_any_element();
            }
            Load::Failed(err) => {
                return list
                    .p_3()
                    .text_sm()
                    .text_color(theme.danger)
                    .child(err.clone())
                    .into_any_element();
            }
            Load::Ready(outgoing) => outgoing,
        };
        let row = |id: ElementId, selected: bool| {
            h_flex()
                .id(id)
                .h(px(ROW_HEIGHT))
                .flex_none()
                .mx_1()
                .px_2()
                .gap_1()
                .rounded_sm()
                .text_sm()
                .cursor_pointer()
                .when(selected, |el| el.bg(theme.list_active))
                .when(!selected, |el| el.hover(|s| s.bg(theme.list_hover)))
        };
        let link = |text: String| div().text_color(theme.primary).child(text);
        let mut header = row("push-branch".into(), self.selected.is_none())
            .child(
                style::lucide("git-branch")
                    .small()
                    .text_color(theme.muted_foreground),
            )
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(outgoing.local.clone()),
            )
            .child(
                Icon::new(IconName::ArrowRight)
                    .xsmall()
                    .text_color(theme.muted_foreground),
            );
        header = match &outgoing.target {
            Some(target) => header
                .child(link(target.remote.clone()))
                .child(":")
                .child(link(target.branch.clone()))
                .when(target.new_branch, |el| {
                    el.child(
                        div()
                            .ml_1()
                            .px_1()
                            .rounded_sm()
                            .text_xs()
                            .bg(theme.success.opacity(0.14))
                            .text_color(theme.success)
                            .child("New"),
                    )
                }),
            None => header.child(div().text_color(theme.muted_foreground).child("no remote")),
        };
        let header = header.on_click(cx.listener(|this, _, _, cx| this.select(None, cx)));
        let commits = outgoing.commits.iter().enumerate().map(|(ix, commit)| {
            row(("push-commit", ix).into(), self.selected == Some(ix))
                .pl(px(8. + INDENT * 1.5))
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(commit.subject.clone()),
                )
                .child(
                    div()
                        .flex_none()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(format!(
                            "{} · {} · {}",
                            commit.short_hash, commit.author, commit.date
                        )),
                )
                .on_click(cx.listener(move |this, _, _, cx| this.select(Some(ix), cx)))
        });
        list.child(header).children(commits).into_any_element()
    }

    fn render_files(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let panel = v_flex()
            .w(px(360.))
            .flex_none()
            .h_full()
            .border_l_1()
            .border_color(theme.border);
        let Some(files) = self.files() else {
            return panel.into_any_element();
        };
        let title = match files.len() {
            1 => "1 file".to_string(),
            n => format!("{n} files"),
        };
        let rows = tree::rows(files, &self.collapsed)
            .into_iter()
            .enumerate()
            .map(|(ix, row)| {
                let base = h_flex()
                    .id(("push-file", ix))
                    .h(px(ROW_HEIGHT))
                    .flex_none()
                    .mx_1()
                    .pr_2()
                    .gap_1p5()
                    .rounded_sm()
                    .text_sm()
                    .hover(|s| s.bg(theme.list_hover));
                match row {
                    TreeRow::Dir {
                        path,
                        label,
                        depth,
                        files,
                        expanded,
                    } => base
                        .pl(px(8. + INDENT * depth as f32))
                        .cursor_pointer()
                        .child(
                            Icon::new(if expanded {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .with_size(style::TREE_CHEVRON)
                            .text_color(muted),
                        )
                        .child(Icon::new(IconName::Folder).small().text_color(muted))
                        .child(label)
                        .child(div().text_xs().text_color(muted).child(files.to_string()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !this.collapsed.remove(&path) {
                                this.collapsed.insert(path.clone());
                            }
                            cx.notify();
                        }))
                        .into_any_element(),
                    TreeRow::File { ix, depth } => {
                        let file = &files[ix];
                        let name = tree::file_name(&file.path);
                        let kind = file.kind();
                        base.pl(px(8. + INDENT * depth as f32 + 14.))
                            .child(file_icons::icon_for_file(name, cx).small())
                            .child(
                                div()
                                    .flex_none()
                                    .text_color(kind_color(kind, cx))
                                    .when(kind == ChangeKind::Deleted, |el| el.line_through())
                                    .child(name.to_string()),
                            )
                            .children(file.orig_path.clone().map(|orig| {
                                div()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .whitespace_nowrap()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(format!("← {orig}"))
                            }))
                            .into_any_element()
                    }
                }
            });
        panel
            .child(
                div()
                    .px_3()
                    .py_1p5()
                    .text_xs()
                    .text_color(muted)
                    .border_b_1()
                    .border_color(theme.border)
                    .child(title),
            )
            .child(
                v_flex()
                    .id("push-files")
                    .flex_1()
                    .py_1()
                    .overflow_y_scroll()
                    .children(rows),
            )
            .into_any_element()
    }
}

impl Render for PushDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.focused_once {
            self.focused_once = true;
            window.focus(&self.focus, cx);
        }
        let theme = cx.theme();
        let border = theme.border;
        let can_push = self.pushable().is_some();
        v_flex()
            .track_focus(&self.focus)
            .capture_key_down(cx.listener(Self::on_key))
            .gap_3()
            .child(self.render_summary(cx))
            .child(
                h_flex()
                    .h(px(BODY_HEIGHT))
                    .border_1()
                    .border_color(border)
                    .rounded(theme.radius)
                    .overflow_hidden()
                    .child(self.render_commits(cx))
                    .child(self.render_files(cx)),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Checkbox::new("push-tags")
                            .label("Push tags")
                            .checked(self.tags)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.tags = *checked;
                                cx.notify();
                            })),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("push-cancel")
                            .ghost()
                            .small()
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("push-confirm")
                            .primary()
                            .small()
                            .label("Push")
                            .disabled(!can_push)
                            .on_click(cx.listener(|this, _, window, cx| this.push(window, cx))),
                    ),
            )
    }
}

impl Workspace {
    /// Opens the Push dialog for `branch`, or for the current branch.
    pub(super) fn open_push_dialog(
        &mut self,
        branch: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(git) = self.git.as_ref() else { return };
        let Some(branch) = branch.or_else(|| git.status.branch.clone()) else {
            window.push_notification(
                Notification::warning("HEAD is detached: check out a branch to push it.").title("Push"),
                cx,
            );
            return;
        };
        let repo = git.repo.clone();
        let project = self
            .root
            .file_name()
            .map_or_default(|n| n.to_string_lossy().into_owned());
        let workspace = cx.entity().downgrade();
        let dialog = cx.new(|cx| PushDialog::new(workspace, repo, branch, cx));
        window.open_dialog(cx, move |d, _, _| {
            d.w(px(900.))
                .title(format!("Push Commits to {project}"))
                .child(dialog.clone())
        });
    }

    fn push_to(
        &mut self,
        local: String,
        target: PushTarget,
        tags: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_git(
            "Push",
            move |repo| repo.push_to(&local, &target, tags),
            |_, summary, window, cx| {
                window.push_notification(Notification::success(summary.message()).title("Push"), cx);
            },
            window,
            cx,
        );
    }
}
