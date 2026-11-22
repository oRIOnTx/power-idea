//! Git Log tool window (⌘9): history with the commit graph, branches and
//! commit details. This module holds the state and loading; see the submodules.

mod branches;
mod details;
mod model;
mod ops;
mod view;

use std::collections::HashSet;
use std::rc::Rc;

use anyhow::Result;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use vcs::{Branch, CommitDetails, GraphRow, HistoryQuery, LogEntry, Operation, Repo};

use super::{BottomPanel, Workspace};
use model::{matches_filter, parse_filter};

/// Commits loaded at first and added by each "Load more".
const HISTORY_PAGE: usize = 2000;
/// All commit-file diffs from the log share one tab.
const LOG_DIFF: &str = "diff:log";

pub(super) struct GitLogState {
    pub(super) entries: Rc<Vec<LogEntry>>,
    pub(super) graph: Rc<Vec<GraphRow>>,
    /// Indices into `entries` that match the filter.
    pub(super) visible: Rc<Vec<usize>>,
    pub(super) branches: Vec<Branch>,
    pub(super) operation: Option<Operation>,
    /// Branch whose history is shown; `None` shows all refs.
    pub(super) branch: Option<String>,
    /// Hash of the selected commit.
    pub(super) selected: Option<String>,
    pub(super) details: Option<CommitDetails>,
    pub(super) filter: Entity<InputState>,
    /// Collapsed sections and folders of the branches panel.
    pub(super) collapsed: HashSet<String>,
    pub(super) scroll: UniformListScrollHandle,
    pub(super) focus: FocusHandle,
    pub(super) loading: bool,
    /// Commits requested from git; more are loaded on demand.
    pub(super) limit: usize,
    /// Filter terms (`path:`, `after:`, `before:`) the loaded history was restricted to.
    pub(super) git_terms: (Option<String>, Option<String>, Option<String>),
    _subscription: Subscription,
}

impl GitLogState {
    pub(super) fn selected_index(&self) -> Option<usize> {
        let hash = self.selected.as_deref()?;
        self.visible.iter().position(|&ix| self.entries[ix].hash == hash)
    }

    fn refilter(&mut self, cx: &App) {
        let filter = parse_filter(&self.filter.read(cx).value());
        self.visible = Rc::new(
            self.entries
                .iter()
                .enumerate()
                .filter(|(_, e)| matches_filter(e, &filter))
                .map(|(ix, _)| ix)
                .collect(),
        );
    }

    /// The graph only makes sense when every commit is shown.
    pub(super) fn show_graph(&self) -> bool {
        self.visible.len() == self.entries.len() && self.git_terms == (None, None, None)
    }

    /// Git returned as many commits as requested, so older ones may exist.
    pub(super) fn has_more(&self) -> bool {
        self.entries.len() >= self.limit
    }
}

struct LogSnapshot {
    entries: Vec<LogEntry>,
    graph: Vec<GraphRow>,
    branches: Vec<Branch>,
    operation: Option<Operation>,
}

fn load_snapshot(repo: &Repo, query: &HistoryQuery) -> Result<LogSnapshot> {
    let entries = repo.history(query)?;
    let graph = vcs::graph_layout(entries.iter().map(|e| (e.hash.as_str(), e.parents.as_slice())));
    Ok(LogSnapshot {
        entries,
        graph,
        branches: repo.branches()?,
        operation: repo.operation_in_progress(),
    })
}

impl Workspace {
    /// ⌘9: show the Git Log, or hide it when it is already shown.
    pub(super) fn toggle_git_log(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.git.is_none() {
            return self.notify_error("Not a git repository".into(), window, cx);
        }
        if self.git_log_visible() {
            self.show_bottom = false;
            self.focus_editor(window, cx);
            return cx.notify();
        }
        self.show_git_log(window, cx);
    }

    pub(super) fn show_git_log(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.git_log.is_none() {
            self.git_log = Some(self.new_git_log_state(window, cx));
        }
        self.show_bottom = true;
        self.bottom_panel = BottomPanel::GitLog;
        if let Some(log) = &self.git_log {
            log.focus.focus(window, cx);
        }
        self.reload_git_log(cx);
    }

    fn new_git_log_state(&mut self, window: &mut Window, cx: &mut Context<Self>) -> GitLogState {
        let filter = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Text or hash · author:name · path:dir · after:2026-01-31")
        });
        let subscription = cx.subscribe(&filter, |this, _, event: &InputEvent, cx| {
            let (InputEvent::Change, Some(log)) = (event, this.git_log.as_mut()) else {
                return;
            };
            // Path and dates are filtered by git, so changing them means a new history.
            let terms = parse_filter(&log.filter.read(cx).value()).git_terms();
            if terms != log.git_terms {
                log.git_terms = terms;
                log.limit = HISTORY_PAGE;
                return this.reload_git_log(cx);
            }
            log.refilter(cx);
            cx.notify();
        });
        GitLogState {
            entries: Rc::default(),
            graph: Rc::default(),
            visible: Rc::default(),
            branches: Vec::new(),
            operation: None,
            branch: None,
            selected: None,
            details: None,
            filter,
            collapsed: HashSet::new(),
            scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            loading: false,
            limit: HISTORY_PAGE,
            git_terms: (None, None, None),
            _subscription: subscription,
        }
    }

    /// Reloads history, branches and the in-progress operation, keeping the selection.
    pub(super) fn reload_git_log(&mut self, cx: &mut Context<Self>) {
        let (Some(git), Some(log)) = (&self.git, self.git_log.as_mut()) else {
            return;
        };
        let repo = git.repo.clone();
        let (path, after, before) = log.git_terms.clone();
        let query = HistoryQuery {
            revision: log.branch.clone(),
            limit: log.limit,
            // `path:` is typed relative to the project; git wants it relative to the repository.
            path: path.map(|p| git.repo_path(&p)),
            after,
            before,
        };
        log.loading = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let snapshot = cx
                .background_spawn(async move { load_snapshot(&repo, &query) })
                .await;
            this.update(cx, |ws, cx| ws.apply_log_snapshot(snapshot, cx)).ok();
        })
        .detach();
    }

    fn apply_log_snapshot(&mut self, snapshot: Result<LogSnapshot>, cx: &mut Context<Self>) {
        let Some(log) = self.git_log.as_mut() else { return };
        log.loading = false;
        let snapshot = match snapshot {
            Ok(snapshot) => snapshot,
            // The filtered branch may have been deleted meanwhile: fall back to all refs.
            Err(_) if log.branch.is_some() => {
                log.branch = None;
                return self.reload_git_log(cx);
            }
            Err(err) => {
                self.pending_notices
                    .push(super::Notice::Error(format!("Git log: {err:#}")));
                return cx.notify();
            }
        };
        log.entries = Rc::new(snapshot.entries);
        log.graph = Rc::new(snapshot.graph);
        log.branches = snapshot.branches;
        log.operation = snapshot.operation;
        log.refilter(cx);
        let keep = log.selected_index().is_some();
        let first = log.visible.first().map(|&ix| log.entries[ix].hash.clone());
        cx.notify();
        match (keep, first) {
            (true, _) => {
                let hash = log.selected.clone();
                self.load_commit_details(hash, cx);
            }
            (false, first) => self.select_commit(first, cx),
        }
    }

    pub(super) fn select_commit(&mut self, hash: Option<String>, cx: &mut Context<Self>) {
        let Some(log) = self.git_log.as_mut() else { return };
        if log.selected == hash && log.details.is_some() {
            return;
        }
        log.selected = hash.clone();
        if let Some(ix) = log.selected_index() {
            log.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
        }
        cx.notify();
        self.load_commit_details(hash, cx);
    }

    fn load_commit_details(&mut self, hash: Option<String>, cx: &mut Context<Self>) {
        let (Some(git), Some(hash)) = (&self.git, hash) else {
            if let Some(log) = self.git_log.as_mut() {
                log.details = None;
            }
            return;
        };
        let repo = git.repo.clone();
        cx.spawn(async move |this, cx| {
            let details = cx
                .background_spawn(async move { repo.commit_details(&hash) })
                .await;
            this.update(cx, |ws, cx| {
                let Some(log) = ws.git_log.as_mut() else { return };
                // Ignore stale results after a quick re-selection.
                if let Ok(details) = details
                    && log.selected.as_ref().is_some_and(|s| *s == details.hash)
                {
                    log.details = Some(details);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Moves the log selection by `delta` rows (arrow keys).
    pub(super) fn step_commit(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(log) = &self.git_log else { return };
        if log.visible.is_empty() {
            return;
        }
        let next = match log.selected_index() {
            Some(ix) => ix.saturating_add_signed(delta).min(log.visible.len() - 1),
            None => 0,
        };
        let hash = log.entries[log.visible[next]].hash.clone();
        self.select_commit(Some(hash), cx);
    }

    /// Loads the next page of older commits.
    pub(super) fn load_more_history(&mut self, cx: &mut Context<Self>) {
        if let Some(log) = self.git_log.as_mut() {
            log.limit += HISTORY_PAGE;
            self.reload_git_log(cx);
        }
    }

    /// Shows the history of `branch` (all refs when `None`).
    pub(super) fn set_log_branch(&mut self, branch: Option<String>, cx: &mut Context<Self>) {
        let Some(log) = self.git_log.as_mut() else { return };
        if log.branch == branch {
            return;
        }
        log.branch = branch;
        self.reload_git_log(cx);
    }

    pub(super) fn toggle_branch_group(&mut self, key: String, cx: &mut Context<Self>) {
        if let Some(log) = self.git_log.as_mut()
            && !log.collapsed.remove(&key)
        {
            log.collapsed.insert(key);
        }
        cx.notify();
    }

    pub(super) fn open_log_file_diff(
        &mut self,
        hash: String,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let short = hash.get(..8).unwrap_or(&hash);
        let name = path.rsplit('/').next().unwrap_or(&path);
        let title = format!("{short} {name}");
        self.open_patch(
            LOG_DIFF.to_string(),
            title,
            None,
            move |repo| repo.commit_file_diff(&hash, &path),
            window,
            cx,
        );
    }

    /// Whether the Git Log is the visible bottom tool window.
    pub(super) fn git_log_visible(&self) -> bool {
        self.show_bottom && self.bottom_panel == BottomPanel::GitLog
    }
}
