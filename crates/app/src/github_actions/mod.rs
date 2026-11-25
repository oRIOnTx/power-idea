//! GitHub Actions tab (one per window): the project's workflow runs with HEAD's status on
//! top, a run's jobs → steps → log, polled with ETags at an adaptive pace.

mod detail;
mod fetch;
mod keys;
mod rows;
mod view;
mod view_detail;
mod view_setup;

use std::path::PathBuf;
use std::time::Duration;

use ci::poller::{Poll, PollInput, next_poll};
use ci::time::{Millis, now_millis};
use ci::{ApiError, GitHub, Run, State, TokenSource};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;

use self::detail::Detail;
use self::fetch::{Setup, SetupResult, Snapshot};
use crate::workspace::Workspace;

pub const CI_CONTEXT: &str = "CiView";

actions!(
    ci,
    [
        Refresh,
        OpenOnGitHub,
        Back,
        OpenSelected,
        FindInLog,
        CopySelection
    ]
);

/// The HEAD status changed (the activity bar badge follows it).
#[derive(Debug, Clone, Copy)]
pub struct HeadChanged;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Runs of the checked-out branch.
    Branch,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pane {
    Runs,
    Detail,
}

pub struct CiView {
    workspace: WeakEntity<Workspace>,
    root: PathBuf,
    focus: FocusHandle,
    setup: Setup,
    client: Option<GitHub>,
    token_source: Option<TokenSource>,
    branch: Option<String>,
    head_sha: Option<String>,
    scope: Scope,
    /// Runs of the current branch (newest first): the HEAD status and the default list.
    branch_runs: Vec<Run>,
    /// Runs of every branch, fetched only while "All" is shown.
    all_runs: Vec<Run>,
    runs_loaded: bool,
    selected_run: Option<u64>,
    /// The user picked a run: stop auto-selecting HEAD's.
    user_selected: bool,
    pane: Pane,
    detail: Option<Detail>,
    error: Option<String>,
    failures: u32,
    blocked_until: Option<i64>,
    last_updated: Option<Millis>,
    fetching: bool,
    /// A refresh was asked for while one was in flight: run it as soon as that one lands.
    refresh_queued: bool,
    visible: bool,
    window_active: bool,
    head_state: Option<State>,
    pat_input: Entity<InputState>,
    saving_pat: bool,
    search: Entity<InputState>,
    search_ix: usize,
    runs_scroll: UniformListScrollHandle,
    detail_scroll: UniformListScrollHandle,
    fetch_task: Option<Task<()>>,
    timer_task: Option<Task<()>>,
    jobs_task: Option<Task<()>>,
    ticker: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<HeadChanged> for CiView {}

/// Refresh right away when the tab comes back after this long.
const STALE_AFTER: Millis = 10_000;

impl CiView {
    pub fn new(
        workspace: WeakEntity<Workspace>,
        root: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let pat_input = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("github_pat_… or ghp_…")
        });
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search in log  (⌘F)"));
        let subscriptions = vec![
            cx.observe_window_activation(window, |this, window, cx| {
                this.set_window_active(window.is_window_active(), cx);
            }),
            cx.subscribe_in(
                &search,
                window,
                |this, _, event: &InputEvent, _, cx| match event {
                    InputEvent::Change => {
                        this.search_ix = 0;
                        this.scroll_to_match(cx);
                        cx.notify();
                    }
                    InputEvent::PressEnter { secondary, .. } => this.step_match(!*secondary, cx),
                    _ => {}
                },
            ),
            cx.subscribe_in(&pat_input, window, |this, _, event: &InputEvent, _, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.save_pat(cx);
                }
            }),
        ];
        let mut view = Self {
            workspace,
            root,
            focus: cx.focus_handle(),
            setup: Setup::Checking,
            client: None,
            token_source: None,
            branch: None,
            head_sha: None,
            scope: Scope::Branch,
            branch_runs: Vec::new(),
            all_runs: Vec::new(),
            runs_loaded: false,
            selected_run: None,
            user_selected: false,
            pane: Pane::Runs,
            detail: None,
            error: None,
            failures: 0,
            blocked_until: None,
            last_updated: None,
            fetching: false,
            refresh_queued: false,
            visible: true,
            window_active: window.is_window_active(),
            head_state: None,
            pat_input,
            saving_pat: false,
            search,
            search_ix: 0,
            runs_scroll: UniformListScrollHandle::new(),
            detail_scroll: UniformListScrollHandle::new(),
            fetch_task: None,
            timer_task: None,
            jobs_task: None,
            ticker: None,
            _subscriptions: subscriptions,
        };
        view.start(cx);
        view
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    /// The HEAD status for the activity bar badge.
    pub fn head_state(&self) -> Option<State> {
        self.head_state
    }

    /// (Re)checks repository, remote and token; connects when all are there.
    fn start(&mut self, cx: &mut Context<Self>) {
        // Replacing `fetch_task` cancels a poll in flight; it will not come back to clear these.
        self.fetching = false;
        self.refresh_queued = false;
        self.timer_task = None;
        self.setup = Setup::Checking;
        cx.notify();
        let root = self.root.clone();
        self.fetch_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { fetch::check_setup(&root) })
                .await;
            this.update(cx, |view, cx| view.setup_checked(result, cx)).ok();
        }));
    }

    fn setup_checked(&mut self, result: SetupResult, cx: &mut Context<Self>) {
        match result {
            SetupResult::Connected { client, source } => {
                self.client = Some(client);
                self.token_source = Some(source);
                self.setup = Setup::Ready;
                self.error = None;
                self.failures = 0;
                self.refresh(cx);
            }
            SetupResult::Unavailable(setup) => {
                self.client = None;
                self.setup = setup;
                self.set_head_state(None, cx);
            }
        }
        cx.notify();
    }

    pub(crate) fn retry_setup(&mut self, cx: &mut Context<Self>) {
        self.start(cx);
    }

    pub(crate) fn save_pat(&mut self, cx: &mut Context<Self>) {
        if self.saving_pat {
            return;
        }
        let token = self.pat_input.read(cx).value().trim().to_string();
        if token.is_empty() {
            return;
        }
        self.saving_pat = true;
        cx.notify();
        self.fetch_task = Some(cx.spawn(async move |this, cx| {
            let saved = cx
                .background_spawn(async move { ci::auth::store_pat(&token) })
                .await;
            this.update(cx, |view, cx| {
                view.saving_pat = false;
                match saved {
                    Ok(()) => view.start(cx),
                    Err(err) => {
                        view.setup = Setup::NotConnected {
                            gh_installed: view.setup.gh_installed(),
                            error: Some(format!("{err:#}")),
                        };
                        cx.notify();
                    }
                }
            })
            .ok();
        }));
    }

    pub(crate) fn forget_pat(&mut self, cx: &mut Context<Self>) {
        self.fetch_task = Some(cx.spawn(async move |this, cx| {
            cx.background_spawn(async { ci::auth::delete_pat() }).await;
            this.update(cx, CiView::start).ok();
        }));
    }

    /// The tab became the active one (or stopped being it).
    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.visible == visible {
            return;
        }
        self.visible = visible;
        self.wake(cx);
    }

    fn set_window_active(&mut self, active: bool, cx: &mut Context<Self>) {
        if self.window_active == active {
            return;
        }
        self.window_active = active;
        self.wake(cx);
    }

    /// Refreshes now if the data is stale, else reschedules for the new pace.
    fn wake(&mut self, cx: &mut Context<Self>) {
        let stale = self.last_updated.is_none_or(|t| now_millis() - t > STALE_AFTER);
        if self.window_active && stale && !self.fetching {
            self.refresh(cx);
        } else {
            self.schedule(cx);
        }
        self.sync_ticker(cx);
    }

    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.setup != Setup::Ready {
            return;
        }
        if self.fetching {
            self.refresh_queued = true;
            return;
        }
        let Some(client) = self.client.clone() else { return };
        self.fetching = true;
        self.timer_task = None;
        cx.notify();
        let root = self.root.clone();
        let want_all = self.scope == Scope::All && self.visible;
        let jobs_of = self
            .detail
            .as_ref()
            .filter(|d| {
                // Until the run and every job have finished, so the last step states are final.
                let active = self.run_is_active(d.run_id) || d.jobs.iter().any(|j| j.state().is_active());
                self.visible && (!d.loaded || active)
            })
            .map(|d| d.run_id);
        self.fetch_task = Some(cx.spawn(async move |this, cx| {
            let snapshot = cx
                .background_spawn(async move { fetch::snapshot(&client, &root, want_all, jobs_of) })
                .await;
            this.update(cx, |view, cx| view.apply_snapshot(snapshot, cx)).ok();
        }));
    }

    fn apply_snapshot(&mut self, snapshot: Snapshot, cx: &mut Context<Self>) {
        self.fetching = false;
        self.last_updated = Some(now_millis());
        let branch_changed = self.branch != snapshot.branch;
        self.branch = snapshot.branch;
        self.head_sha = snapshot.head_sha;
        let mut error = None;
        match snapshot.branch_runs {
            Some(Ok(fetched)) => {
                if fetched.changed || branch_changed {
                    self.branch_runs = fetched.value;
                }
            }
            Some(Err(err)) => error = Some(err),
            None => self.branch_runs.clear(),
        }
        match snapshot.all_runs {
            Some(Ok(fetched)) if fetched.changed => self.all_runs = fetched.value,
            Some(Err(err)) if error.is_none() => error = Some(err),
            _ => {}
        }
        if let Some((run_id, result)) = snapshot.jobs {
            match result {
                Ok(fetched) => self.jobs_loaded(run_id, fetched.value, cx),
                Err(err) if error.is_none() => error = Some(err),
                Err(_) => {}
            }
        }
        if let Some(err) = error {
            self.poll_failed(&err, cx);
        } else {
            self.failures = 0;
            self.blocked_until = None;
            self.error = None;
            self.runs_loaded = true;
        }
        let head = self
            .head_sha
            .as_deref()
            .and_then(|sha| rows::head_state(&self.branch_runs, sha));
        self.set_head_state(head, cx);
        self.auto_select(cx);
        if std::mem::take(&mut self.refresh_queued) {
            self.refresh(cx);
        } else {
            self.schedule(cx);
        }
        self.sync_ticker(cx);
        cx.notify();
    }

    fn poll_failed(&mut self, err: &ApiError, cx: &mut Context<Self>) {
        self.failures = self.failures.saturating_add(1);
        match err {
            ApiError::Unauthorized => {
                self.client = None;
                self.setup = Setup::NotConnected {
                    gh_installed: self.setup.gh_installed() || self.token_source == Some(TokenSource::GhCli),
                    error: Some(err.to_string()),
                };
                self.set_head_state(None, cx);
            }
            ApiError::RateLimited { reset_at } => {
                self.blocked_until = Some(reset_at.unwrap_or(now_millis() / 1000 + 60));
            }
            _ => {}
        }
        if err.is_unexpected() {
            error_journal::report_message(&err.to_string(), "GitHub Actions");
        }
        self.error = Some(err.to_string());
    }

    fn set_head_state(&mut self, state: Option<State>, cx: &mut Context<Self>) {
        if self.head_state != state {
            self.head_state = state;
            cx.emit(HeadChanged);
        }
    }

    /// Schedules the next poll (or none while paused).
    fn schedule(&mut self, cx: &mut Context<Self>) {
        if self.fetching || self.setup != Setup::Ready {
            self.timer_task = None;
            return;
        }
        let input = PollInput {
            window_active: self.window_active,
            panel_visible: self.visible,
            any_active: self.any_active(),
            failures: self.failures,
            rate: self.client.as_ref().and_then(GitHub::rate_limit),
            blocked_until: self.blocked_until,
            now_secs: now_millis() / 1000,
        };
        self.timer_task = match next_poll(&input) {
            Poll::Paused => None,
            Poll::After(wait) => Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(wait).await;
                this.update(cx, CiView::refresh).ok();
            })),
        };
    }

    /// Repaints every second while the tab is on screen (ticking timers, "updated Xs ago").
    fn sync_ticker(&mut self, cx: &mut Context<Self>) {
        let wanted = self.visible && self.window_active && self.setup == Setup::Ready;
        if !wanted {
            self.ticker = None;
            return;
        }
        if self.ticker.is_some() {
            return;
        }
        self.ticker = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        }));
    }

    fn any_active(&self) -> bool {
        self.shown_runs().iter().any(|r| r.state().is_active())
            || self
                .detail
                .as_ref()
                .is_some_and(|d| d.jobs.iter().any(|j| j.state().is_active()))
    }

    fn run_is_active(&self, run_id: u64) -> bool {
        self.find_run(run_id).is_some_and(|r| r.state().is_active())
    }

    pub(crate) fn shown_runs(&self) -> &[Run] {
        match self.scope {
            Scope::All => &self.all_runs,
            Scope::Branch => &self.branch_runs,
        }
    }

    fn find_run(&self, run_id: u64) -> Option<&Run> {
        self.branch_runs
            .iter()
            .chain(&self.all_runs)
            .find(|r| r.id == run_id)
    }

    pub(crate) fn selected_run(&self) -> Option<&Run> {
        self.find_run(self.selected_run?)
    }

    pub(crate) fn set_scope(&mut self, scope: Scope, cx: &mut Context<Self>) {
        if self.scope == scope {
            return;
        }
        self.scope = scope;
        self.last_updated = None;
        self.refresh(cx);
        cx.notify();
    }

    /// Opens HEAD's run (a failed one first) until the user picks a run themselves.
    fn auto_select(&mut self, cx: &mut Context<Self>) {
        if self.user_selected {
            return;
        }
        let head = self.head_sha.clone().unwrap_or_default();
        let runs = self.shown_runs();
        let pick = runs
            .iter()
            .find(|r| r.head_sha == head && r.state() == State::Failure)
            .or_else(|| runs.iter().find(|r| r.head_sha == head))
            .or_else(|| runs.first())
            .map(|r| r.id);
        if let Some(id) = pick
            && self.selected_run != Some(id)
        {
            self.select_run(id, false, cx);
        }
    }
}
