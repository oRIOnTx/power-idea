//! Opening the Push dialog and running the push it confirms.

use std::fmt::Write as _;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;
use vcs::PushTarget;

use super::PushDialog;
use crate::style;
use crate::workspace::Workspace;

impl Workspace {
    /// Opens the Push dialog for `branch`, or for the current branch.
    pub(in crate::workspace) fn open_push_dialog(
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

    /// Force push asks first: it can drop commits others pushed.
    pub(super) fn confirm_force_push(
        &mut self,
        local: String,
        target: PushTarget,
        tags: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let workspace = cx.entity().downgrade();
        let name = target.name();
        let protected = matches!(target.branch.as_str(), "main" | "master");
        window.open_alert_dialog(cx, move |dialog, _, cx| {
            let (workspace, local, target) = (workspace.clone(), local.clone(), target.clone());
            let mut body = format!(
                "{name} is replaced with {local}; commits only the remote has are lost. \
                 The push is refused if someone pushed to {name} since your last fetch."
            );
            if protected {
                let _ = write!(body, " {} is usually a shared branch.", target.branch);
            }
            style::danger_confirm(dialog, "triangle-alert", "Force Push", cx)
                .title(format!("Force push {local} to {name}?"))
                .child(body)
                .on_ok(move |_, window, cx| {
                    let (local, target) = (local.clone(), target.clone());
                    workspace
                        .update(cx, |ws, cx| ws.push_to(local, target, tags, true, window, cx))
                        .ok();
                    true
                })
        });
    }

    pub(super) fn push_to(
        &mut self,
        local: String,
        target: PushTarget,
        tags: bool,
        force: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_git(
            if force { "Force Push" } else { "Push" },
            move |repo| repo.push_to(&local, &target, tags, force),
            |_, summary, window, cx| {
                window.push_notification(Notification::success(summary.message()).title("Push"), cx);
            },
            window,
            cx,
        );
    }
}
