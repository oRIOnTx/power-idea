//! Branch tools from the Branches popup and the log's branch menu: New Worktree,
//! Show Diff with Working Tree and the tracked (upstream) branch.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use vcs::Branch;

use super::model::check_ref_name;
use crate::style;
use crate::workspace::Workspace;

/// What a worktree starts from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Start {
    /// A local branch not checked out anywhere here: the worktree checks it out.
    FreeLocal,
    Remote,
    /// The current branch, a revision or HEAD.
    Other,
}

/// Where a worktree for `start` goes by default, and the new branch it gets (none for a
/// free local branch); a branch can be checked out in only one worktree.
pub(super) fn worktree_defaults(root: &Path, start: &str, kind: Start) -> (PathBuf, String) {
    let branch = match kind {
        Start::FreeLocal => String::new(),
        // origin/feature → a local feature, as JetBrains does.
        Start::Remote => start.split_once('/').map_or(start, |(_, rest)| rest).to_string(),
        Start::Other => format!("{start}-worktree"),
    };
    let project = root
        .file_name()
        .map_or_default(|n| n.to_string_lossy().into_owned());
    let suffix = if branch.is_empty() { start } else { &branch }.replace('/', "-");
    let location = root.parent().unwrap_or(root).join(format!("{project}-{suffix}"));
    (location, branch)
}

fn check_location(text: &str) -> Result<PathBuf> {
    let path = PathBuf::from(text.trim());
    if !path.is_absolute() {
        bail!("Enter an absolute folder path");
    }
    if path.read_dir().is_ok_and(|mut dir| dir.next().is_some()) {
        bail!("{} is not empty", path.display());
    }
    Ok(path)
}

impl Workspace {
    /// New Worktree (from `start`, or from the current branch): asks where and on which branch,
    /// then opens the worktree as a project.
    pub(in crate::workspace) fn prompt_new_worktree(
        &mut self,
        start: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(git) = self.git.as_ref() else { return };
        let repo = git.repo.clone();
        let start = start
            .or_else(|| git.status.branch.clone())
            .unwrap_or_else(|| "HEAD".into());
        let root = self.root.clone();
        cx.spawn_in(window, async move |this, cx| {
            let branches = cx.background_spawn(async move { repo.branches() }).await;
            this.update_in(cx, |ws, window, cx| {
                let branches = branches.unwrap_or_default();
                let kind = match branches.iter().find(|b| b.name == start) {
                    Some(b) if b.is_remote => Start::Remote,
                    Some(b) if !b.is_current => Start::FreeLocal,
                    _ => Start::Other,
                };
                let (location, branch) = worktree_defaults(&root, &start, kind);
                ws.open_worktree_dialog(start, location, branch, window, cx);
            })
            .ok();
        })
        .detach();
    }

    fn open_worktree_dialog(
        &mut self,
        start: String,
        location: PathBuf,
        branch: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = |value: String, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| InputState::new(window, cx).default_value(value))
        };
        let location = input(location.to_string_lossy().into_owned(), window, cx);
        let branch = input(branch, window, cx);
        let error: Entity<Option<String>> = cx.new(|_| None);
        let submit = {
            let workspace = cx.entity().downgrade();
            let (location, branch, error, start) =
                (location.clone(), branch.clone(), error.clone(), start.clone());
            std::rc::Rc::new(move |window: &mut Window, cx: &mut App| {
                let new_branch = branch.read(cx).value().trim().to_string();
                let checked = check_location(&location.read(cx).value()).and_then(|path| {
                    let name = (!new_branch.is_empty())
                        .then(|| check_ref_name(&new_branch))
                        .transpose()?;
                    Ok((path, name))
                });
                let (path, name) = match checked {
                    Ok(ok) => ok,
                    Err(err) => {
                        error.update(cx, |e, cx| {
                            *e = Some(format!("{err:#}"));
                            cx.notify();
                        });
                        return;
                    }
                };
                window.close_dialog(cx);
                let start = start.clone();
                workspace
                    .update(cx, |ws, cx| ws.add_worktree(path, start, name, window, cx))
                    .ok();
            })
        };
        for input in [&location, &branch] {
            let submit = submit.clone();
            style::submit_on_enter(input, window, cx, move |window, cx| submit(window, cx));
        }
        let focused = std::cell::Cell::new(false);
        window.open_dialog(cx, move |dialog, window, cx| {
            if !focused.replace(true) {
                branch.update(cx, |i, cx| i.focus(window, cx));
            }
            let theme = cx.theme();
            let label = |text: &'static str| div().text_sm().text_color(theme.muted_foreground).child(text);
            let submit = submit.clone();
            dialog
                .w(px(560.))
                .title(format!("New Worktree from '{start}'"))
                .child(
                    v_flex()
                        .gap_2()
                        .child(label("Location"))
                        .child(Input::new(&location))
                        .child(label("New branch (leave empty to check out the branch itself)"))
                        .child(Input::new(&branch))
                        .children(
                            error
                                .read(cx)
                                .clone()
                                .map(|m| div().text_sm().text_color(theme.danger).child(m)),
                        ),
                )
                .footer(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .justify_end()
                        .child(
                            Button::new("cancel")
                                .ghost()
                                .small()
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("create-worktree")
                                .primary()
                                .small()
                                .label("Create")
                                .on_click(move |_, window, cx| submit(window, cx)),
                        ),
                )
        });
    }

    fn add_worktree(
        &mut self,
        path: PathBuf,
        start: String,
        new_branch: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = path.clone();
        self.run_git(
            "New Worktree",
            move |repo| repo.add_worktree(&target, &start, new_branch.as_deref()),
            move |_, _, window, cx| {
                let text = format!("Created worktree {}", path.display());
                window.push_notification(Notification::success(text).title("New Worktree"), cx);
                crate::projects::open_project(path, cx);
            },
            window,
            cx,
        );
    }

    /// All differences between `revision` and the files on disk, in one diff tab.
    pub(in crate::workspace) fn show_diff_with_working_tree(
        &mut self,
        revision: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_patch(
            format!("diff:working-tree:{revision}"),
            format!("{revision} ↔ Working Tree"),
            None,
            move |repo| repo.diff_with_working_tree(&revision),
            window,
            cx,
        );
    }

    /// Picks the remote branch `local` tracks, or none.
    pub(in crate::workspace) fn open_tracked_branch_dialog(
        &mut self,
        local: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(repo) = self.git.as_ref().map(|g| g.repo.clone()) else {
            return;
        };
        cx.spawn_in(window, async move |this, cx| {
            let branches = cx.background_spawn(async move { repo.branches() }).await;
            this.update_in(cx, |ws, window, cx| match branches {
                Ok(branches) => ws.show_tracked_branch_dialog(local, branches, window, cx),
                Err(err) => window.push_notification(Notification::error(format!("{err:#}")), cx),
            })
            .ok();
        })
        .detach();
    }

    fn show_tracked_branch_dialog(
        &mut self,
        local: String,
        branches: Vec<Branch>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = branches
            .iter()
            .find(|b| b.name == local && !b.is_remote)
            .and_then(|b| b.upstream.clone());
        let choices: Vec<Option<String>> = branches
            .into_iter()
            .filter(|b| b.is_remote)
            .map(|b| Some(b.name))
            .chain([None])
            .collect();
        let workspace = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            let theme = cx.theme();
            let rows = choices.iter().enumerate().map(|(ix, choice)| {
                let selected = *choice == current;
                let (workspace, local, choice) = (workspace.clone(), local.clone(), choice.clone());
                h_flex()
                    .id(("tracked", ix))
                    .h(px(28.))
                    .px_2()
                    .gap_2()
                    .rounded(theme.radius)
                    .text_sm()
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.list_hover))
                    .when(selected, |el| el.bg(theme.list_active))
                    .child(div().w(px(16.)).flex_none().when(selected, |el| {
                        el.child(Icon::new(IconName::Check).small().text_color(theme.primary))
                    }))
                    .child(match &choice {
                        Some(name) => h_flex()
                            .gap_2()
                            .child(
                                style::lucide("git-branch")
                                    .small()
                                    .text_color(theme.muted_foreground),
                            )
                            .child(name.clone()),
                        None => h_flex()
                            .text_color(theme.muted_foreground)
                            .child("None (don't track a remote branch)"),
                    })
                    .on_click(move |_, window, cx| {
                        window.close_dialog(cx);
                        let (local, choice) = (local.clone(), choice.clone());
                        workspace
                            .update(cx, |ws, cx| ws.set_tracked_branch(local, choice, window, cx))
                            .ok();
                    })
            });
            dialog
                .w(px(420.))
                .title(format!("Tracked Branch for '{local}'"))
                .child(
                    v_flex()
                        .id("tracked-branches")
                        .max_h(px(360.))
                        .overflow_y_scroll()
                        .children(rows),
                )
        });
    }

    fn set_tracked_branch(
        &mut self,
        local: String,
        upstream: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let done = match &upstream {
            Some(upstream) => format!("{local} now tracks {upstream}"),
            None => format!("{local} no longer tracks a remote branch"),
        };
        self.run_log_op(
            "Tracked Branch",
            done,
            move |repo| repo.set_upstream(&local, upstream.as_deref()),
            window,
            cx,
        );
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{Start, check_location, worktree_defaults};

    #[test]
    fn worktree_defaults_name_the_folder_after_the_branch() {
        let root = Path::new("/dev/power-idea");
        assert_eq!(
            worktree_defaults(root, "main", Start::Other),
            (
                PathBuf::from("/dev/power-idea-main-worktree"),
                "main-worktree".into()
            )
        );
        assert_eq!(
            worktree_defaults(root, "feature/login", Start::FreeLocal),
            (PathBuf::from("/dev/power-idea-feature-login"), String::new())
        );
        assert_eq!(
            worktree_defaults(root, "origin/feature/login", Start::Remote),
            (
                PathBuf::from("/dev/power-idea-feature-login"),
                "feature/login".into()
            )
        );
    }

    #[test]
    fn worktree_location_must_be_a_new_or_empty_folder() {
        assert!(check_location("relative/dir").is_err());
        assert!(check_location("/").is_err(), "not empty");
        assert!(check_location("/surely/missing/power-idea-wt").is_ok());
    }
}
