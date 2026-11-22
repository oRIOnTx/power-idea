//! Git integration for the workspace: file status colors, the Commit tool
//! window, diffs, push/pull and branches. All git calls run in the background.

use std::collections::HashSet;
use std::time::Duration;

use gpui_kit::component::diff::{DiffFile, DiffMode, DiffState};
use gpui_kit::component::input::{InputEvent, TextareaState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme as _, WindowExt as _};
use gpui_kit::*;
use vcs::{Branch, ChangeKind, FileChange, Repo, Status};

use super::Workspace;
use crate::editor::{CenterTab, DiffTab};
use crate::palette::PaletteMode;

const REFRESH_INTERVAL: Duration = Duration::from_secs(3);
const LOCAL_CHANGES_DIFF: &str = "diff:local";

pub struct GitState {
    pub(super) repo: Repo,
    /// Project root relative to the repo root, `""` or ending with `/`.
    pub(super) prefix: String,
    pub status: Status,
    /// Repo paths selected for the next commit.
    pub(super) checked: HashSet<String>,
    /// Paths seen in the last status, to auto-check only new changes.
    pub(super) known: HashSet<String>,
    pub(super) message: Entity<TextareaState>,
    pub(super) amend: bool,
    pub(super) busy: Option<&'static str>,
    /// Collapsed change groups in the Commit panel.
    pub(super) collapsed: HashSet<&'static str>,
    pub(super) _subscription: Subscription,
}

impl GitState {
    /// Repo path of a project-relative path.
    pub(super) fn repo_path(&self, rel: &str) -> String {
        format!("{}{rel}", self.prefix)
    }

    /// Status of a project-relative file, or whether a directory contains changes.
    pub fn kind_for(&self, rel: &str, is_dir: bool) -> Option<ChangeKind> {
        let path = self.repo_path(rel);
        if is_dir {
            let dir = format!("{path}/");
            return self
                .status
                .changes
                .iter()
                .any(|c| c.path.starts_with(&dir) && c.kind != ChangeKind::Deleted)
                .then_some(ChangeKind::Modified);
        }
        self.status
            .changes
            .iter()
            .find(|c| c.path == path)
            .map(|c| c.kind)
    }
}

pub fn kind_color(kind: ChangeKind, cx: &App) -> Hsla {
    let theme = cx.theme();
    match kind {
        ChangeKind::Modified | ChangeKind::Renamed => theme.blue,
        ChangeKind::Added => theme.green,
        ChangeKind::Untracked => theme.red_light,
        ChangeKind::Deleted => theme.muted_foreground,
        ChangeKind::Conflicted => theme.red,
    }
}

pub(super) fn kind_letter(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Modified => "M",
        ChangeKind::Added => "A",
        ChangeKind::Deleted => "D",
        ChangeKind::Renamed => "R",
        ChangeKind::Untracked => "U",
        ChangeKind::Conflicted => "C",
    }
}

impl Workspace {
    pub(super) fn init_git(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repo) = Repo::discover(&self.root) else {
            return;
        };
        let prefix = self
            .root
            .canonicalize()
            .ok()
            .and_then(|root| root.strip_prefix(repo.root()).ok().map(|p| p.to_path_buf()))
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .filter(|p| !p.is_empty())
            .map_or_default(|p| format!("{p}/"));

        let message = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Commit message")
                .rows(4)
        });
        let subscription = cx.subscribe_in(&message, window, |this, _, event: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { secondary: true, .. } = event {
                this.commit(false, window, cx);
            }
        });
        self.git = Some(GitState {
            repo,
            prefix,
            status: Status::default(),
            checked: HashSet::new(),
            known: HashSet::new(),
            message,
            amend: false,
            busy: None,
            collapsed: HashSet::new(),
            _subscription: subscription,
        });

        cx.spawn(async move |this, cx| {
            loop {
                let Ok(Some(repo)) = this.read_with(cx, |ws, _| ws.git.as_ref().map(|g| g.repo.clone()))
                else {
                    break;
                };
                let status = cx.background_spawn(async move { repo.status() }).await;
                if this.update(cx, |ws, cx| ws.apply_status(status, cx)).is_err() {
                    break;
                }
                cx.background_executor().timer(REFRESH_INTERVAL).await;
            }
        })
        .detach();
    }

    pub(super) fn refresh_git(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.git.as_ref().map(|g| g.repo.clone()) else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let status = cx.background_spawn(async move { repo.status() }).await;
            this.update(cx, |ws, cx| ws.apply_status(status, cx)).ok();
        })
        .detach();
    }

    fn apply_status(&mut self, status: anyhow::Result<Status>, cx: &mut Context<Self>) {
        let (Some(git), Ok(status)) = (self.git.as_mut(), status) else {
            return;
        };
        if git.status == status {
            return;
        }
        let paths: HashSet<String> = status.changes.iter().map(|c| c.path.clone()).collect();
        // New changes are included in the commit by default, like the JetBrains default changelist.
        for path in paths.difference(&git.known) {
            git.checked.insert(path.clone());
        }
        git.checked.retain(|p| paths.contains(p));
        git.known = paths;
        // Commits, checkouts and pulls made outside the IDE show up as a branch or ahead/behind change.
        let moved = (&git.status.branch, git.status.ahead, git.status.behind)
            != (&status.branch, status.ahead, status.behind);
        git.status = status;
        if moved && self.git_log_visible() {
            self.reload_git_log(cx);
        }
        cx.notify();
    }

    /// Runs a git operation in the background, reports the result and refreshes status.
    pub(super) fn run_git<T: Send + 'static>(
        &mut self,
        label: &'static str,
        op: impl FnOnce(Repo) -> anyhow::Result<T> + Send + 'static,
        on_success: impl FnOnce(&mut Self, T, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(git) = self.git.as_mut() else { return };
        if git.busy.is_some() {
            return;
        }
        git.busy = Some(label);
        let repo = git.repo.clone();
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx.background_spawn(async move { op(repo) }).await;
            this.update_in(cx, |ws, window, cx| {
                if let Some(git) = ws.git.as_mut() {
                    git.busy = None;
                }
                match result {
                    Ok(value) => on_success(ws, value, window, cx),
                    Err(err) => window.push_notification(
                        Notification::error(format!("{err:#}")).title(format!("{label} failed")),
                        cx,
                    ),
                }
                ws.refresh_git(cx);
                ws.reload_git_log(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn commit(&mut self, push: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(git) = self.git.as_ref() else { return };
        let message = git.message.read(cx).value().to_string();
        let amend = git.amend;
        let paths: Vec<String> = git
            .status
            .changes
            .iter()
            .filter(|c| git.checked.contains(&c.path))
            .flat_map(|c| c.orig_path.iter().cloned().chain([c.path.clone()]))
            .collect();
        let label = if push { "Commit and Push" } else { "Commit" };
        self.run_git(
            label,
            move |repo| {
                repo.commit(&message, &paths, amend)?;
                push.then(|| repo.push_with_summary()).transpose()
            },
            move |ws, pushed, window, cx| {
                if let Some(git) = ws.git.as_mut() {
                    git.amend = false;
                    git.message.update(cx, |m, cx| m.set_value("", window, cx));
                }
                let text =
                    pushed.map_or_else(|| "Committed".into(), |p| format!("Committed. {}", p.message()));
                window.push_notification(Notification::success(text), cx);
            },
            window,
            cx,
        );
    }

    pub(super) fn push(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run_git(
            "Push",
            |repo| repo.push_with_summary(),
            |_, summary, window, cx| {
                window.push_notification(Notification::success(summary.message()).title("Push"), cx);
            },
            window,
            cx,
        );
    }

    /// JetBrains "Update Project".
    pub(super) fn pull(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run_git(
            "Update",
            |repo| repo.pull_with_summary(),
            |ws, summary, window, cx| {
                ws.reload_clean_editors(window, cx);
                ws.reindex(cx);
                window.push_notification(
                    Notification::success(summary.message()).title("Update Project"),
                    cx,
                );
            },
            window,
            cx,
        );
    }

    pub(super) fn set_amend(&mut self, amend: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(git) = self.git.as_mut() else { return };
        git.amend = amend;
        if amend && git.message.read(cx).value().trim().is_empty() {
            let repo = git.repo.clone();
            let message = git.message.clone();
            cx.spawn_in(window, async move |_, cx| {
                if let Ok(text) = cx
                    .background_spawn(async move { repo.last_commit_message() })
                    .await
                {
                    message
                        .update_in(cx, |m, window, cx| m.set_value(text, window, cx))
                        .ok();
                }
            })
            .detach();
        }
        cx.notify();
    }

    pub(super) fn rollback(&mut self, change: FileChange, window: &mut Window, cx: &mut Context<Self>) {
        let workspace = cx.entity().downgrade();
        let path = change.path.clone();
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let workspace = workspace.clone();
            let change = change.clone();
            dialog
                .title("Rollback changes?")
                .child(format!("All local changes in {path} will be lost."))
                .on_ok(move |_, window, cx| {
                    let change = change.clone();
                    workspace
                        .update(cx, |ws, cx| {
                            ws.run_git(
                                "Rollback",
                                move |repo| repo.rollback(&change),
                                |ws, _, window, cx| ws.reload_clean_editors(window, cx),
                                window,
                                cx,
                            );
                        })
                        .ok();
                    true
                })
        });
    }

    /// Reloads editors without unsaved edits after git changed files on disk.
    pub(super) fn reload_clean_editors(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for tab in &mut self.tabs {
            let CenterTab::Editor(tab) = tab else { continue };
            if tab.is_dirty(cx) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&tab.path) else {
                continue;
            };
            if text.as_str() != tab.saved_text.as_ref() {
                tab.state
                    .update(cx, |s, cx| s.set_value(text.clone(), window, cx));
                tab.saved_text = text.into();
            }
        }
    }

    pub(super) fn open_change_diff(
        &mut self,
        change: FileChange,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // All local-change diffs share one tab, like the JetBrains diff viewer.
        let key = LOCAL_CHANGES_DIFF.to_string();
        let title = format!("Δ {}", change.path.rsplit('/').next().unwrap_or(&change.path));
        let path = Some(change.path.clone());
        self.open_patch(key, title, path, move |repo| repo.diff_file(&change), window, cx);
    }

    pub(super) fn open_patch(
        &mut self,
        key: String,
        title: String,
        change_path: Option<String>,
        load: impl FnOnce(Repo) -> anyhow::Result<String> + Send + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(repo) = self.git.as_ref().map(|g| g.repo.clone()) else {
            return;
        };
        cx.spawn_in(window, async move |this, cx| {
            let files = cx
                .background_spawn(async move {
                    let patch = load(repo)?;
                    DiffFile::parse(&patch).map_err(|err| anyhow::anyhow!("{err:?}"))
                })
                .await;
            this.update_in(cx, |ws, window, cx| match files {
                Ok(files) if files.is_empty() => {
                    window.push_notification(Notification::info("No changes"), cx);
                }
                Ok(files) => ws.show_diff_tab(key, title, change_path, files, window, cx),
                Err(err) => window.push_notification(Notification::error(format!("{err:#}")), cx),
            })
            .ok();
        })
        .detach();
    }

    fn show_diff_tab(
        &mut self,
        key: String,
        title: String,
        change_path: Option<String>,
        files: Vec<DiffFile>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let existing = self
            .tabs
            .iter()
            .position(|t| matches!(t, CenterTab::Diff(d) if d.key == key));
        let ix = match existing {
            Some(ix) => {
                if let CenterTab::Diff(tab) = &mut self.tabs[ix] {
                    tab.title = title;
                    tab.change_path = change_path;
                    tab.state.update(cx, |state, cx| state.set_files(files, cx));
                }
                ix
            }
            None => {
                let state = cx.new(|cx| DiffState::new(files, cx).with_mode(DiffMode::Split));
                self.tabs.push(CenterTab::Diff(DiffTab {
                    key,
                    title,
                    change_path,
                    state,
                }));
                self.tabs.len() - 1
            }
        };
        self.activate_tab(ix, window, cx);
    }

    pub(super) fn open_branches(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repo) = self.git.as_ref().map(|g| g.repo.clone()) else {
            return;
        };
        cx.spawn_in(window, async move |this, cx| {
            let branches = cx.background_spawn(async move { repo.branches() }).await;
            this.update_in(cx, |ws, window, cx| match branches {
                Ok(branches) => ws.open_palette_with(PaletteMode::Branches, branches, window, cx),
                Err(err) => window.push_notification(Notification::error(format!("{err:#}")), cx),
            })
            .ok();
        })
        .detach();
    }

    pub fn checkout(&mut self, branch: Branch, window: &mut Window, cx: &mut Context<Self>) {
        let name = branch.name.clone();
        self.run_git(
            "Checkout",
            move |repo| repo.checkout(&branch),
            move |ws, _, window, cx| {
                ws.reload_clean_editors(window, cx);
                ws.reindex(cx);
                window.push_notification(Notification::success(format!("Checked out {name}")), cx);
            },
            window,
            cx,
        );
    }

    pub fn create_branch(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let label = name.clone();
        self.run_git(
            "New Branch",
            move |repo| repo.create_branch(&name),
            move |_, _, window, cx| {
                window.push_notification(Notification::success(format!("Created branch {label}")), cx);
            },
            window,
            cx,
        );
    }

    pub(super) fn focus_commit_message(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(git) = &self.git {
            git.message.update(cx, |m, cx| m.focus(window, cx));
        }
    }

    pub(super) fn fetch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run_git(
            "Fetch",
            |repo| repo.fetch_with_summary(),
            |_, summary, window, cx| {
                window.push_notification(Notification::info(summary.message()).title("Fetch"), cx);
            },
            window,
            cx,
        );
    }

    /// Opens a changed file (repo path) in the editor, if it is inside the project.
    pub(super) fn open_repo_file(&mut self, repo_path: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(git) = &self.git else { return };
        if let Some(rel) = repo_path.strip_prefix(git.prefix.as_str()) {
            let rel = rel.to_string();
            self.open_file(&rel, None, window, cx);
        }
    }

    pub(super) fn set_checked(&mut self, paths: Vec<String>, checked: bool, cx: &mut Context<Self>) {
        if let Some(git) = self.git.as_mut() {
            for path in paths {
                if checked {
                    git.checked.insert(path);
                } else {
                    git.checked.remove(&path);
                }
            }
        }
        cx.notify();
    }

    /// Opens the diff of the previous/next local change relative to `current`.
    pub(super) fn step_change(
        &mut self,
        current: &str,
        forward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(git) = &self.git else { return };
        let changes = &git.status.changes;
        if changes.is_empty() {
            return;
        }
        let ix = changes.iter().position(|c| c.path == current).unwrap_or(0);
        let next = if forward {
            (ix + 1) % changes.len()
        } else {
            (ix + changes.len() - 1) % changes.len()
        };
        let change = changes[next].clone();
        self.open_change_diff(change, window, cx);
    }
}
