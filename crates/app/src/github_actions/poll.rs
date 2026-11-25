//! Polling of the GitHub Actions tab: when to ask again, what a failed poll means, the
//! one-second repaint while on screen, and notifications for watched runs that finished.

use std::collections::HashSet;
use std::time::Duration;

use ci::poller::{Poll, PollInput, next_poll};
use ci::time::now_millis;
use ci::{ApiError, GitHub, Run, State, TokenSource, watch};
use gpui_kit::*;

use super::fetch::Setup;
use super::{CiEvent, CiView, notify};

impl CiView {
    pub(super) fn poll_failed(&mut self, err: &ApiError, cx: &mut Context<Self>) {
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

    pub(super) fn set_head_state(&mut self, state: Option<State>, cx: &mut Context<Self>) {
        if self.head_state != state {
            self.head_state = state;
            cx.emit(CiEvent::HeadChanged);
        }
    }

    /// Schedules the next poll (or none while paused).
    pub(super) fn schedule(&mut self, cx: &mut Context<Self>) {
        if self.fetching || self.setup != Setup::Ready {
            self.timer_task = None;
            return;
        }
        let input = PollInput {
            window_active: self.window_active,
            panel_visible: self.visible,
            any_active: self.any_active(),
            watching: self.watching(),
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
    pub(super) fn sync_ticker(&mut self, cx: &mut Context<Self>) {
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

    pub(super) fn any_active(&self) -> bool {
        self.shown_runs().iter().any(|r| r.state().is_active())
            || self
                .detail
                .as_ref()
                .is_some_and(|d| d.jobs.iter().any(|j| j.state().is_active()))
    }

    /// Every run known, freshest copy first (the branch list is polled even when hidden).
    pub(super) fn known_runs(&self) -> Vec<Run> {
        let mut seen = HashSet::new();
        self.tracked_runs
            .iter()
            .chain(&self.branch_runs)
            .chain(&self.all_runs)
            .filter(|r| seen.insert(r.id))
            .cloned()
            .collect()
    }

    pub(super) fn is_watched(&self, run: &Run) -> bool {
        self.triggered.contains(&run.id) || self.head_sha.as_deref() == Some(run.head_sha.as_str())
    }

    /// A run on HEAD or one started from the IDE is still going.
    pub(super) fn watching(&self) -> bool {
        self.known_runs()
            .iter()
            .any(|r| r.state().is_active() && self.is_watched(r))
    }

    /// Emits a notification for each watched run that finished since the previous poll.
    pub(super) fn notify_finished(&mut self, cx: &mut Context<Self>) {
        let runs = self.known_runs();
        let finished: Vec<Run> = watch::finished(&self.previous, &runs, |r| self.is_watched(r))
            .into_iter()
            .filter(|r| self.notified.insert((r.id, r.run_attempt)))
            .cloned()
            .collect();
        self.previous = watch::states(&runs);
        for run in finished {
            let (title, body) = watch::summary(&run);
            let failed = run.state() == State::Failure;
            if !self.window_active {
                notify::os_notification(&title, &body, failed);
            }
            cx.emit(CiEvent::Notify { title, body, failed });
        }
    }
}
