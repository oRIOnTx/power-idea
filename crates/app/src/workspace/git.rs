//! Git integration for the workspace: file status colors, the Commit tool
//! window, diffs, push/pull and branches. All git calls run in the background.

use std::collections::HashSet;
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::diff::{DiffFile, DiffMode, DiffState};
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, WindowExt as _};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use vcs::{Branch, ChangeKind, Commit, FileChange, Repo, Status};

use super::Workspace;
use crate::editor::{CenterTab, DiffTab};
use crate::palette::PaletteMode;
use crate::style;

const REFRESH_INTERVAL: Duration = Duration::from_secs(3);
const LOG_LIMIT: usize = 500;
const LOCAL_CHANGES_DIFF: &str = "diff:local";

pub struct GitState {
    pub(super) repo: Repo,
    /// Project root relative to the repo root, `""` or ending with `/`.
    pub(super) prefix: String,
    pub status: Status,
    /// Repo paths selected for the next commit.
    checked: HashSet<String>,
    /// Paths seen in the last status, to auto-check only new changes.
    known: HashSet<String>,
    message: Entity<TextareaState>,
    amend: bool,
    busy: Option<&'static str>,
    /// Collapsed change groups in the Commit panel.
    collapsed: HashSet<&'static str>,
    _subscription: Subscription,
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

fn kind_letter(kind: ChangeKind) -> &'static str {
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
            .map(|p| format!("{p}/"))
            .unwrap_or_default();

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
        git.status = status;
        cx.notify();
    }

    /// Runs a git operation in the background, reports the result and refreshes status.
    fn run_git<T: Send + 'static>(
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
                if push {
                    repo.push()?;
                }
                Ok(())
            },
            move |ws, _, window, cx| {
                if let Some(git) = ws.git.as_mut() {
                    git.amend = false;
                    git.message.update(cx, |m, cx| m.set_value("", window, cx));
                }
                let text = if push { "Committed and pushed" } else { "Committed" };
                window.push_notification(Notification::success(text), cx);
            },
            window,
            cx,
        );
    }

    pub(super) fn push(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run_git(
            "Push",
            |repo| repo.push(),
            |_, _, window, cx| window.push_notification(Notification::success("Pushed"), cx),
            window,
            cx,
        );
    }

    /// JetBrains "Update Project".
    pub(super) fn pull(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run_git(
            "Update",
            |repo| repo.pull(),
            |ws, _, window, cx| {
                ws.reload_clean_editors(window, cx);
                ws.reindex(cx);
                window.push_notification(Notification::success("Project updated"), cx)
            },
            window,
            cx,
        );
    }

    fn set_amend(&mut self, amend: bool, window: &mut Window, cx: &mut Context<Self>) {
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

    fn rollback(&mut self, change: FileChange, window: &mut Window, cx: &mut Context<Self>) {
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
                            )
                        })
                        .ok();
                    true
                })
        });
    }

    /// Reloads editors without unsaved edits after git changed files on disk.
    fn reload_clean_editors(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    pub fn open_commit(&mut self, commit: Commit, window: &mut Window, cx: &mut Context<Self>) {
        let key = format!("commit:{}", commit.hash);
        let title = format!("{} {}", commit.short_hash, commit.subject);
        self.open_patch(
            key,
            title,
            None,
            move |repo| repo.show_commit(&commit.hash),
            window,
            cx,
        );
    }

    fn open_patch(
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
                Ok(branches) => ws.open_palette_with(PaletteMode::Branches, branches, Vec::new(), window, cx),
                Err(err) => window.push_notification(Notification::error(format!("{err:#}")), cx),
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn open_git_log(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repo) = self.git.as_ref().map(|g| g.repo.clone()) else {
            return;
        };
        cx.spawn_in(window, async move |this, cx| {
            let log = cx.background_spawn(async move { repo.log(LOG_LIMIT) }).await;
            this.update_in(cx, |ws, window, cx| match log {
                Ok(log) => ws.open_palette_with(PaletteMode::GitLog, Vec::new(), log, window, cx),
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
                window.push_notification(Notification::success(format!("Created branch {label}")), cx)
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

    pub(super) fn render_branch_label(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let git = self.git.as_ref()?;
        let status = &git.status;
        let mut label = format!("⎇ {}", status.branch.as_deref().unwrap_or("detached"));
        if status.ahead > 0 {
            label.push_str(&format!(" ↑{}", status.ahead));
        }
        if status.behind > 0 {
            label.push_str(&format!(" ↓{}", status.behind));
        }
        Some(
            div()
                .id("branch")
                .px_1()
                .rounded_sm()
                .cursor_pointer()
                .text_color(cx.theme().primary)
                .hover(|s| s.bg(cx.theme().muted))
                .child(label)
                .on_click(cx.listener(|ws, _, window, cx| ws.open_branches(window, cx)))
                .into_any_element(),
        )
    }

    fn fetch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run_git("Fetch", |repo| repo.fetch(), |_, _, _, _| {}, window, cx);
    }

    /// Opens a changed file (repo path) in the editor, if it is inside the project.
    fn open_repo_file(&mut self, repo_path: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(git) = &self.git else { return };
        if let Some(rel) = repo_path.strip_prefix(git.prefix.as_str()) {
            let rel = rel.to_string();
            self.open_file(&rel, None, window, cx);
        }
    }

    fn set_checked(&mut self, paths: Vec<String>, checked: bool, cx: &mut Context<Self>) {
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

    fn render_branch_card(&self, git: &GitState, cx: &mut Context<Self>) -> impl IntoElement {
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
                            .on_click(cx.listener(|ws, _, window, cx| ws.open_git_log(window, cx))),
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

    fn render_change_row(
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
            .px_2()
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
            .child(Checkbox::new(("check", ix)).checked(checked).on_click(
                cx.listener(move |ws, checked: &bool, _, cx| {
                    ws.set_checked(vec![path.clone()], *checked, cx)
                }),
            ))
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
                                ws.open_repo_file(&open_file, window, cx)
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
                                ws.rollback(rollback.clone(), window, cx)
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
                    ws.open_repo_file(&open_diff.path, window, cx)
                } else {
                    ws.open_change_diff(open_diff.clone(), window, cx)
                }
            }))
    }

    fn render_change_group(
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

    fn render_commit_box(&self, git: &GitState, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let selected = git.checked.len();
        let busy = git.busy.is_some();
        let message = git.message.read(cx).value();
        let subject_len = message.lines().next().map(|l| l.chars().count()).unwrap_or(0);
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

    /// Opens the diff of the previous/next local change relative to `current`.
    fn step_change(&mut self, current: &str, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
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
                            .on_click(
                                cx.listener(move |ws, _, window, cx| {
                                    ws.step_change(&prev, false, window, cx)
                                }),
                            ),
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
