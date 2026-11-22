//! The Branches popup's life cycle: open it under the branch widget, load branches, and
//! carry out what was picked.

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;

use super::Workspace;
use crate::actions::{GitLogAction, GitLogOp};
use crate::branches_popup::{BranchesPopup, BranchesPopupEvent};

/// How many recently checked-out branches the "Recent" group shows.
const RECENT_BRANCHES: usize = 5;

impl Workspace {
    pub(super) fn branches_popup(&self) -> Option<Entity<BranchesPopup>> {
        self.branches_popup.as_ref().map(|(popup, _)| popup.clone())
    }

    /// Opens the popup (or closes it when it is open, like clicking the widget again).
    pub(super) fn open_branches(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.branches_popup.is_some() {
            return self.close_branches(cx);
        }
        let Some(repo) = self.git.as_ref().map(|g| g.repo.clone()) else {
            return;
        };
        let popup = cx.new(|cx| BranchesPopup::new(window, cx));
        let subscription = cx.subscribe_in(&popup, window, Self::on_branches_event);
        self.branches_popup = Some((popup.clone(), subscription));
        cx.notify();

        cx.spawn_in(window, async move |this, cx| {
            let loaded = cx
                .background_spawn(async move {
                    let branches = repo.branches()?;
                    // A repository without commits has no reflog yet; Recent is just empty.
                    let recent = repo.recent_branches(RECENT_BRANCHES).unwrap_or_default();
                    anyhow::Ok((branches, recent))
                })
                .await;
            this.update_in(cx, |ws, window, cx| match loaded {
                Ok((branches, recent)) => {
                    popup.update(cx, |popup, cx| popup.set_branches(branches, recent, cx));
                }
                Err(err) => {
                    ws.close_branches(cx);
                    window.push_notification(Notification::error(format!("{err:#}")), cx);
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn close_branches(&mut self, cx: &mut Context<Self>) {
        if self.branches_popup.take().is_some() {
            cx.notify();
        }
    }

    fn on_branches_event(
        &mut self,
        _: &Entity<BranchesPopup>,
        event: &BranchesPopupEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_branches(cx);
        match event {
            BranchesPopupEvent::Dismiss => {}
            BranchesPopupEvent::Run(action) => self.dispatch_deferred(action.boxed_clone(), window, cx),
            BranchesPopupEvent::Create(name) => {
                self.prompt_new_branch("HEAD".into(), name.clone(), window, cx);
            }
            BranchesPopupEvent::Branch(GitLogOp::CheckoutBranch, branch) => {
                self.checkout(branch.clone(), window, cx);
            }
            BranchesPopupEvent::Branch(GitLogOp::DeleteBranch, branch) => {
                self.delete_branch(branch.name.clone(), branch.is_remote, window, cx);
            }
            BranchesPopupEvent::Branch(op, branch) => {
                let action = GitLogAction {
                    op: *op,
                    target: branch.name.clone().into(),
                };
                self.handle_git_log_action(&action, window, cx);
            }
        }
    }
}
