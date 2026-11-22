//! Git Log operations on commits and branches (cherry-pick, reset, merge, …).

use std::rc::Rc;

use anyhow::Result;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;
use vcs::{Repo, ResetMode};

use super::model::check_ref_name;
use crate::actions::{GitLogAction, GitLogOp};
use crate::workspace::Workspace;

impl Workspace {
    /// Runs an operation that changes the working tree, then reloads editors and the index.
    fn run_log_op<T: Send + 'static>(
        &mut self,
        label: &'static str,
        done: String,
        op: impl FnOnce(Repo) -> Result<T> + Send + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_git(
            label,
            op,
            move |ws, _, window, cx| {
                ws.reload_clean_editors(window, cx);
                ws.reindex(cx);
                window.push_notification(Notification::success(done), cx);
            },
            window,
            cx,
        );
    }

    fn confirm(
        &mut self,
        title: String,
        body: String,
        window: &mut Window,
        cx: &mut Context<Self>,
        on_ok: impl Fn(&mut Workspace, &mut Window, &mut Context<Workspace>) + 'static,
    ) {
        let workspace = cx.entity().downgrade();
        let on_ok = Rc::new(on_ok);
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let (workspace, on_ok) = (workspace.clone(), on_ok.clone());
            dialog
                .title(title.clone())
                .child(body.clone())
                .on_ok(move |_, window, cx| {
                    workspace.update(cx, |ws, cx| on_ok(ws, window, cx)).ok();
                    true
                })
        });
    }

    /// Asks for a name, then creates a branch at `start` and checks it out.
    pub(in crate::workspace) fn prompt_new_branch(
        &mut self,
        start: String,
        initial: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prompt_name("New Branch", &initial, window, cx, move |ws, name, window, cx| {
            let name = check_ref_name(&name)?;
            let start = start.clone();
            let done = format!("Created and checked out {name}");
            ws.run_log_op(
                "New Branch",
                done,
                move |repo| repo.create_branch_at(&name, &start, true),
                window,
                cx,
            );
            Ok(())
        });
    }

    /// Asks for a tag, branch or commit and checks it out as a detached HEAD.
    pub(in crate::workspace) fn prompt_checkout_revision(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prompt_name(
            "Checkout Tag or Revision",
            "",
            window,
            cx,
            |ws, revision, window, cx| {
                let revision = revision.trim().to_string();
                if revision.is_empty() {
                    anyhow::bail!("Enter a tag, branch or commit");
                }
                ws.run_log_op(
                    "Checkout",
                    format!("Checked out {revision} (detached HEAD)"),
                    move |repo| repo.checkout_revision(&revision),
                    window,
                    cx,
                );
                Ok(())
            },
        );
    }

    fn current_branch(&self) -> Option<String> {
        self.git.as_ref()?.status.branch.clone()
    }

    pub(in crate::workspace) fn handle_git_log_action(
        &mut self,
        action: &GitLogAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = action.target.to_string();
        let short = target.get(..8).unwrap_or(&target).to_string();
        match action.op {
            GitLogOp::Refresh => self.reload_git_log(cx),
            GitLogOp::CopyRevision => {
                cx.write_to_clipboard(ClipboardItem::new_string(target));
                window.push_notification(Notification::info("Revision number copied"), cx);
            }
            GitLogOp::NewBranchAt => self.prompt_new_branch(target, String::new(), window, cx),
            GitLogOp::NewTagAt => self.prompt_name("New Tag", "", window, cx, move |ws, name, window, cx| {
                let name = check_ref_name(&name)?;
                let hash = target.clone();
                let done = format!("Created tag {name}");
                ws.run_git(
                    "New Tag",
                    move |repo| repo.create_tag(&name, &hash),
                    move |_, _, window, cx| window.push_notification(Notification::success(done), cx),
                    window,
                    cx,
                );
                Ok(())
            }),
            GitLogOp::CheckoutRevision => self.run_log_op(
                "Checkout",
                format!("Checked out {short} (detached HEAD)"),
                move |repo| repo.checkout_revision(&target),
                window,
                cx,
            ),
            GitLogOp::CherryPick => self.run_log_op(
                "Cherry-pick",
                format!("Cherry-picked {short}"),
                move |repo| repo.cherry_pick(&target),
                window,
                cx,
            ),
            GitLogOp::Revert => self.run_log_op(
                "Revert",
                format!("Reverted {short}"),
                move |repo| repo.revert(&target),
                window,
                cx,
            ),
            GitLogOp::ResetSoft | GitLogOp::ResetMixed => {
                let mode = if action.op == GitLogOp::ResetSoft {
                    ResetMode::Soft
                } else {
                    ResetMode::Mixed
                };
                self.run_log_op(
                    "Reset",
                    format!("Reset to {short}"),
                    move |repo| repo.reset(&target, mode),
                    window,
                    cx,
                );
            }
            GitLogOp::ResetHard => {
                let branch = self.current_branch().unwrap_or_else(|| "HEAD".into());
                self.confirm(
                    format!("Hard reset {branch} to {short}?"),
                    "All uncommitted changes and later commits on this branch will be lost.".into(),
                    window,
                    cx,
                    move |ws, window, cx| {
                        let target = target.clone();
                        ws.run_log_op(
                            "Reset",
                            format!("Reset to {short}"),
                            move |repo| repo.reset(&target, ResetMode::Hard),
                            window,
                            cx,
                        );
                    },
                );
            }
            GitLogOp::CheckoutBranch => {
                let branch = self
                    .git_log
                    .as_ref()
                    .and_then(|log| log.branches.iter().find(|b| b.name == target).cloned());
                if let Some(branch) = branch {
                    self.checkout(branch, window, cx);
                }
            }
            GitLogOp::MergeIntoCurrent => {
                let current = self.current_branch().unwrap_or_else(|| "HEAD".into());
                self.run_log_op(
                    "Merge",
                    format!("Merged {target} into {current}"),
                    move |repo| repo.merge(&target),
                    window,
                    cx,
                );
            }
            GitLogOp::RebaseCurrentOnto => {
                let current = self.current_branch().unwrap_or_else(|| "HEAD".into());
                self.run_log_op(
                    "Rebase",
                    format!("Rebased {current} onto {target}"),
                    move |repo| repo.rebase(&target),
                    window,
                    cx,
                );
            }
            GitLogOp::RenameBranch => {
                let old = target.clone();
                self.prompt_name(
                    "Rename Branch",
                    &target,
                    window,
                    cx,
                    move |ws, name, window, cx| {
                        let name = check_ref_name(&name)?;
                        let (old, done) = (old.clone(), format!("Renamed {old} to {name}"));
                        ws.run_git(
                            "Rename Branch",
                            move |repo| repo.rename_branch(&old, &name),
                            move |_, _, window, cx| window.push_notification(Notification::success(done), cx),
                            window,
                            cx,
                        );
                        Ok(())
                    },
                );
            }
            GitLogOp::DeleteBranch => {
                let remote = self
                    .git_log
                    .as_ref()
                    .is_some_and(|log| log.branches.iter().any(|b| b.name == target && b.is_remote));
                self.delete_branch(target, remote, window, cx);
            }
            GitLogOp::PushBranch => self.run_git(
                "Push",
                {
                    let name = target.clone();
                    move |repo| repo.push_branch(&name)
                },
                move |_, _, window, cx| {
                    window.push_notification(Notification::success(format!("Pushed {target}")), cx);
                },
                window,
                cx,
            ),
            GitLogOp::ContinueOperation | GitLogOp::AbortOperation => {
                let live = self.git.as_ref().and_then(|g| g.operation);
                let Some(operation) = live.or_else(|| self.git_log.as_ref().and_then(|log| log.operation))
                else {
                    return;
                };
                if action.op == GitLogOp::ContinueOperation {
                    self.run_log_op(
                        "Continue",
                        format!("{} completed", operation.label()),
                        move |repo| repo.continue_operation(operation),
                        window,
                        cx,
                    );
                } else {
                    self.confirm(
                        format!("Abort {}?", operation.label().to_lowercase()),
                        "Conflict resolutions made so far will be discarded.".into(),
                        window,
                        cx,
                        move |ws, window, cx| {
                            ws.run_log_op(
                                "Abort",
                                format!("{} aborted", operation.label()),
                                move |repo| repo.abort_operation(operation),
                                window,
                                cx,
                            );
                        },
                    );
                }
            }
        }
    }

    /// Deletes a branch after confirmation; offers a force delete when it is not merged.
    pub(in crate::workspace) fn delete_branch(
        &mut self,
        name: String,
        remote: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let body = if remote {
            "The branch is deleted on the remote server.".to_string()
        } else {
            "The local branch is deleted; commits stay reachable from other branches.".to_string()
        };
        self.confirm(
            format!("Delete branch {name}?"),
            body,
            window,
            cx,
            move |ws, window, cx| {
                let target = name.clone();
                let done = format!("Deleted {name}");
                ws.run_git(
                    "Delete Branch",
                    move |repo| {
                        if remote {
                            return repo.delete_remote_branch(&target).map(|_| true);
                        }
                        match repo.delete_branch(&target, false) {
                            Ok(_) => Ok(true),
                            Err(err) if format!("{err:#}").contains("not fully merged") => Ok(false),
                            Err(err) => Err(err),
                        }
                    },
                    {
                        let name = name.clone();
                        move |ws, deleted, window, cx| {
                            if deleted {
                                window.push_notification(Notification::success(done), cx);
                            } else {
                                ws.force_delete_branch(name, window, cx);
                            }
                        }
                    },
                    window,
                    cx,
                );
            },
        );
    }

    fn force_delete_branch(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm(
            format!("{name} is not fully merged"),
            "Delete it anyway? Its unmerged commits will only be reachable through the reflog.".into(),
            window,
            cx,
            move |ws, window, cx| {
                let (target, done) = (name.clone(), format!("Deleted {name}"));
                ws.run_git(
                    "Delete Branch",
                    move |repo| repo.delete_branch(&target, true),
                    move |_, _, window, cx| window.push_notification(Notification::success(done), cx),
                    window,
                    cx,
                );
            },
        );
    }
}
