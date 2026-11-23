//! Push dialog (⌘⇧K), as in JetBrains: before anything leaves the machine it shows where
//! it goes (remote, URL, branch) and what goes there (commits and the files they touch).

mod files;
mod push;
mod tree;

use std::collections::{HashMap, HashSet};

use gpui_kit::component::button::{Button, ButtonVariants as _, DropdownButton};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use vcs::{CommitFile, Outgoing, PushTarget, Repo};

use super::Workspace;
use crate::style;

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
    /// The local branch being pushed.
    branch: String,
    load: Load,
    /// The remote branch name while it is being edited.
    editing: Option<(Entity<InputState>, Subscription)>,
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
    pub(super) fn new(
        workspace: WeakEntity<Workspace>,
        repo: Repo,
        branch: String,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self {
            workspace,
            repo,
            branch,
            load: Load::Loading,
            editing: None,
            selected: None,
            commit_files: HashMap::new(),
            collapsed: HashSet::new(),
            tags: false,
            focus: cx.focus_handle(),
            focused_once: false,
            _tasks: Vec::new(),
        };
        this.load(None, cx);
        this
    }

    /// Loads what goes to `target` (remote, branch), or to the branch's default target.
    // reason: `_`-prefixed fields keep a task/subscription alive; reassigning them is the intent.
    #[allow(clippy::used_underscore_binding)]
    fn load(&mut self, target: Option<(String, String)>, cx: &mut Context<Self>) {
        let (repo, branch) = (self.repo.clone(), self.branch.clone());
        self.load = Load::Loading;
        self.selected = None;
        cx.notify();
        self._tasks = vec![cx.spawn(async move |this, cx| {
            let outgoing = cx
                .background_spawn(async move {
                    match target {
                        Some((remote, remote_branch)) => repo.outgoing_to(&branch, &remote, &remote_branch),
                        None => repo.outgoing(&branch),
                    }
                })
                .await;
            this.update(cx, |this, cx| {
                this.load = match outgoing {
                    Ok(outgoing) => Load::Ready(outgoing),
                    Err(err) => Load::Failed(format!("{err:#}")),
                };
                cx.notify();
            })
            .ok();
        })];
    }

    fn target(&self) -> Option<&PushTarget> {
        self.outgoing()?.target.as_ref()
    }

    fn set_remote(&mut self, remote: String, cx: &mut Context<Self>) {
        let branch = self
            .target()
            .map_or_else(|| self.branch.clone(), |t| t.branch.clone());
        self.load(Some((remote, branch)), cx);
    }

    /// Turns the remote branch name into a text field, as JetBrains does on click.
    fn edit_branch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.target() else { return };
        let initial = target.branch.clone();
        let input = cx.new(|cx| InputState::new(window, cx).default_value(initial));
        input.update(cx, |input, cx| input.focus(window, cx));
        let subscription = cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                this.apply_branch(window, cx);
            }
        });
        self.editing = Some((input, subscription));
        cx.notify();
    }

    fn apply_branch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((input, _)) = self.editing.take() else {
            return;
        };
        window.focus(&self.focus, cx);
        let name = input.read(cx).value().trim().to_string();
        let Some(target) = self.target() else { return };
        if name.is_empty() || name == target.branch {
            return cx.notify();
        }
        let remote = target.remote.clone();
        self.load(Some((remote, name)), cx);
    }

    fn outgoing(&self) -> Option<&Outgoing> {
        match &self.load {
            Load::Ready(outgoing) => Some(outgoing),
            _ => None,
        }
    }

    /// What the Push button would push to, if pushing makes sense; a force push makes sense
    /// whenever there is a target (e.g. to overwrite a remote branch after a rebase).
    fn pushable(&self, force: bool) -> Option<(&Outgoing, &PushTarget)> {
        let outgoing = self.outgoing()?;
        let target = outgoing.target.as_ref()?;
        let anything = !outgoing.commits.is_empty() || target.new_branch || self.tags || force;
        (anything && self.editing.is_none()).then_some((outgoing, target))
    }

    // reason: `_`-prefixed fields keep a task/subscription alive; reassigning them is the intent.
    #[allow(clippy::used_underscore_binding)]
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

    fn push(&mut self, force: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some((outgoing, target)) = self.pushable(force) else {
            return;
        };
        let (local, target, tags) = (outgoing.local.clone(), target.clone(), self.tags);
        window.close_dialog(cx);
        self.workspace
            .update(cx, |ws, cx| {
                if force {
                    ws.confirm_force_push(local, target, tags, window, cx);
                } else {
                    ws.push_to(local, target, tags, false, window, cx);
                }
            })
            .ok();
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let modifiers = event.keystroke.modifiers;
        // The branch field takes its own keys (Enter applies the name).
        if self.editing.is_some() || modifiers.control || modifiers.alt || modifiers.shift {
            return;
        }
        match event.keystroke.key.as_str() {
            "up" if !modifiers.platform => self.step(false, cx),
            "down" if !modifiers.platform => self.step(true, cx),
            "enter" => self.push(false, window, cx),
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
                            "{} has {} commit{} that {} lacks: the push will be rejected. Update the project first, or Force Push.",
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
        let mut remote = self
            .outgoing()
            .and_then(|o| Some(self.render_remote(o, o.target.as_ref()?, cx)));
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
        let link = |id: &'static str, text: String| {
            div()
                .id(id)
                .text_color(theme.primary)
                .cursor_pointer()
                .hover(Styled::underline)
                .child(text)
        };
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
                .children(remote.take())
                .child(":")
                .child(match &self.editing {
                    Some((input, _)) => div()
                        .w(px(180.))
                        .child(Input::new(input).xsmall())
                        .into_any_element(),
                    None => link("push-target-branch", target.branch.clone())
                        .on_click(cx.listener(|this, _, window, cx| {
                            cx.stop_propagation();
                            this.edit_branch(window, cx);
                        }))
                        .into_any_element(),
                })
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

    /// The remote, a menu of all remotes when there are several.
    fn render_remote(&self, outgoing: &Outgoing, target: &PushTarget, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        if outgoing.remotes.len() < 2 {
            return div()
                .text_color(theme.primary)
                .child(target.remote.clone())
                .into_any_element();
        }
        let (remotes, current, this) = (
            outgoing.remotes.clone(),
            target.remote.clone(),
            cx.entity().downgrade(),
        );
        Button::new("push-remote")
            .ghost()
            .xsmall()
            .label(target.remote.clone())
            .dropdown_caret(true)
            .dropdown_menu(move |mut menu, _, _| {
                for remote in &remotes {
                    let (this, remote) = (this.clone(), remote.clone());
                    menu = menu.item(
                        PopupMenuItem::new(remote.clone())
                            .checked(remote == current)
                            .on_click(move |_, _, cx| {
                                this.update(cx, |this, cx| this.set_remote(remote.clone(), cx))
                                    .ok();
                            }),
                    );
                }
                menu
            })
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
        let can_push = self.pushable(false).is_some();
        let can_force = self.pushable(true).is_some();
        let this = cx.entity().downgrade();
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
                        DropdownButton::new("push-confirm")
                            .primary()
                            .small()
                            .button(
                                Button::new("push-confirm-button")
                                    .label("Push")
                                    .disabled(!can_push)
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.push(false, window, cx)),
                                    ),
                            )
                            .disabled(!can_force)
                            .dropdown_menu(move |menu, _, _| {
                                let this = this.clone();
                                menu.item(PopupMenuItem::new("Force Push…").on_click(move |_, window, cx| {
                                    this.update(cx, |this, cx| this.push(true, window, cx)).ok();
                                }))
                            }),
                    ),
            )
    }
}
