//! Conflicts tab (JetBrains "Resolve Conflicts" + merge tool): the conflicted files
//! of a merge/rebase/cherry-pick, per-chunk choices and whole-file resolutions.

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;
use vcs::{ChangeKind, ConflictSegment, Resolution, Side, conflict_count, parse_conflicts, resolve};

use super::Workspace;
use crate::actions::{GitLogAction, GitLogOp};
use crate::editor::CenterTab;

#[derive(Default)]
pub(super) struct ConflictsState {
    /// Repo path of the file in the merge panel.
    pub(super) selected: Option<String>,
    pub(super) segments: Vec<ConflictSegment>,
    /// One per conflict chunk; `None` = not decided yet.
    pub(super) choices: Vec<Option<Resolution>>,
    pub(super) error: Option<String>,
}

impl ConflictsState {
    pub(super) fn resolved_count(&self) -> usize {
        self.choices.iter().filter(|c| c.is_some()).count()
    }
}

/// The path after `current` in `paths` (wrapping), or the first one.
fn next_path(paths: &[String], current: &str) -> Option<String> {
    let Some(ix) = paths.iter().position(|p| p == current) else {
        return paths.first().cloned();
    };
    paths[ix + 1..].iter().chain(&paths[..ix]).next().cloned()
}

impl Workspace {
    /// Repo paths of the files still in conflict.
    pub(super) fn conflicted_paths(&self) -> Vec<String> {
        self.git.as_ref().map_or_default(|git| {
            git.status
                .changes
                .iter()
                .filter(|c| c.kind == ChangeKind::Conflicted)
                .map(|c| c.path.clone())
                .collect()
        })
    }

    fn conflicts_tab_open(&self) -> bool {
        self.tabs.iter().any(|t| matches!(t, CenterTab::Conflicts))
    }

    /// Opens (or focuses) the Conflicts tab, showing `path` or the first conflicted file.
    pub(super) fn open_conflicts(
        &mut self,
        path: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ix = match self.tabs.iter().position(|t| matches!(t, CenterTab::Conflicts)) {
            Some(ix) => ix,
            None => {
                self.tabs.push(CenterTab::Conflicts);
                self.tabs.len() - 1
            }
        };
        self.activate_tab(ix, window, cx);
        match path.or_else(|| self.conflicted_paths().into_iter().next()) {
            Some(path) => self.select_conflict(path, cx),
            None => cx.notify(),
        }
    }

    /// Loads `path` (repo-relative) into the merge panel.
    pub(super) fn select_conflict(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(repo) = self.git.as_ref().map(|g| g.repo.clone()) else {
            return;
        };
        self.conflicts = ConflictsState {
            selected: Some(path.clone()),
            ..Default::default()
        };
        cx.notify();
        cx.spawn(async move |this, cx| {
            let file = repo.root().join(&path);
            let text = cx
                .background_spawn(async move { std::fs::read_to_string(file) })
                .await;
            this.update(cx, |ws, cx| {
                if ws.conflicts.selected.as_deref() != Some(path.as_str()) {
                    return;
                }
                match text {
                    Ok(text) => {
                        let segments = parse_conflicts(&text);
                        ws.conflicts.choices = vec![None; conflict_count(&segments)];
                        ws.conflicts.segments = segments;
                    }
                    // Deleted on one side, or binary: only whole-file choices apply.
                    Err(err) => ws.conflicts.error = Some(format!("{err}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Keeps the merge panel on a conflicted file as the status changes.
    pub(super) fn sync_conflicts(&mut self, cx: &mut Context<Self>) {
        if !self.conflicts_tab_open() {
            return;
        }
        let paths = self.conflicted_paths();
        let current = self.conflicts.selected.clone();
        if current.as_ref().is_some_and(|c| paths.contains(c)) {
            return;
        }
        match paths.into_iter().next() {
            Some(path) => self.select_conflict(path, cx),
            None if current.is_some() => self.conflicts = Default::default(),
            None => {}
        }
    }

    /// Picks (or, clicked again, clears) the resolution of chunk `ix`.
    pub(super) fn choose_resolution(&mut self, ix: usize, resolution: Resolution, cx: &mut Context<Self>) {
        if let Some(choice) = self.conflicts.choices.get_mut(ix) {
            *choice = if *choice == Some(resolution) {
                None
            } else {
                Some(resolution)
            };
            cx.notify();
        }
    }

    /// Writes the merged text of the selected file and stages it.
    pub(super) fn apply_merge(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.conflicts.selected.clone() else {
            return;
        };
        if self.conflicts.choices.iter().any(Option::is_none) {
            return;
        }
        let text = resolve(&self.conflicts.segments, &self.conflicts.choices);
        self.finish_conflict(
            path.clone(),
            "Resolve",
            move |repo| {
                std::fs::write(repo.root().join(&path), text)?;
                repo.mark_resolved(&path)
            },
            window,
            cx,
        );
    }

    pub(super) fn accept_conflict_side(&mut self, side: Side, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.conflicts.selected.clone() else {
            return;
        };
        self.finish_conflict(
            path.clone(),
            "Accept",
            move |repo| repo.accept_side(&path, side),
            window,
            cx,
        );
    }

    /// Stages a file whose markers were removed by hand in the editor.
    pub(super) fn mark_conflict_resolved(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.conflicts.selected.clone() else {
            return;
        };
        self.finish_conflict(
            path.clone(),
            "Mark as Resolved",
            move |repo| {
                let text = std::fs::read_to_string(repo.root().join(&path)).unwrap_or_default();
                let left = conflict_count(&parse_conflicts(&text));
                if left > 0 {
                    anyhow::bail!("{path} still has {left} conflict(s) with markers; save the editor first");
                }
                repo.mark_resolved(&path)
            },
            window,
            cx,
        );
    }

    /// Runs a resolution of `path`, then moves on to the next conflicted file.
    fn finish_conflict(
        &mut self,
        path: String,
        label: &'static str,
        op: impl FnOnce(vcs::Repo) -> anyhow::Result<()> + Send + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_git(
            label,
            op,
            move |ws, (), window, cx| {
                ws.reload_clean_editors(window, cx);
                let paths = ws.conflicted_paths();
                let remaining = paths.iter().filter(|p| **p != path).count();
                let text = match remaining {
                    0 => "All conflicts resolved".to_string(),
                    n => format!("Resolved {path} · {n} left"),
                };
                window.push_notification(Notification::success(text), cx);
                match next_path(&paths, &path) {
                    Some(next) => ws.select_conflict(next, cx),
                    None => ws.conflicts = Default::default(),
                }
            },
            window,
            cx,
        );
    }

    pub(super) fn open_conflict_in_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.conflicts.selected.clone() else {
            return;
        };
        self.open_repo_file(&path, window, cx);
    }

    /// Continue / Abort of the merge, rebase, cherry-pick or revert.
    pub(super) fn conflict_operation(&mut self, op: GitLogOp, window: &mut Window, cx: &mut Context<Self>) {
        let action = GitLogAction {
            op,
            target: "".into(),
        };
        self.handle_git_log_action(&action, window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::next_path;

    #[test]
    fn picks_the_next_conflicted_file() {
        let paths: Vec<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
        assert_eq!(next_path(&paths, "a").as_deref(), Some("b"));
        assert_eq!(next_path(&paths, "c").as_deref(), Some("a"));
        assert_eq!(next_path(&paths, "x").as_deref(), Some("a"));
        assert_eq!(next_path(&["a".to_string()], "a"), None);
    }
}
