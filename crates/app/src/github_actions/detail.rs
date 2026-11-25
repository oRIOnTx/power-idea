//! The selected run: its jobs, which jobs and steps are unfolded, their logs and annotations,
//! log search and line selection.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use ci::{ApiError, Job, State};
use gpui_kit::*;

use super::rows::{self, DetailRow, DetailSource, Expansion, LogState};
use super::{CiView, Pane};

/// Arrow keys through the run list load the run under the cursor once it settles.
const SELECT_DEBOUNCE: Duration = Duration::from_millis(200);
/// A just-finished job's log can 404 for a few seconds.
const LOG_RETRIES: u8 = 3;
const LOG_RETRY_DELAY: Duration = Duration::from_secs(5);

/// The selected run's jobs and whatever was loaded for them.
pub(super) struct Detail {
    pub(super) run_id: u64,
    pub(super) jobs: Vec<Job>,
    pub(super) loaded: bool,
    pub(super) error: Option<String>,
    pub(super) expansion: Expansion,
    /// (job id, state) pairs already auto-unfolded, so a user's fold sticks.
    pub(super) unfolded: HashSet<(u64, State)>,
    pub(super) logs: HashMap<u64, LogState>,
    pub(super) annotations: HashMap<u64, Vec<ci::Annotation>>,
    /// Log fetches that found nothing yet, per job (GitHub writes the log a moment after the end).
    pub(super) log_retries: HashMap<u64, u8>,
    /// Keyboard cursor: an index into the detail rows.
    pub(super) cursor: Option<usize>,
    /// Selected log lines: job index and (anchor, end) line numbers.
    pub(super) selection: Option<(usize, usize, usize)>,
    /// Scroll to the first error of this job once its log is in.
    pub(super) scroll_to_error: Option<u64>,
}

impl Detail {
    pub(super) fn new(run_id: u64) -> Self {
        Self {
            run_id,
            jobs: Vec::new(),
            loaded: false,
            error: None,
            expansion: Expansion::default(),
            unfolded: HashSet::new(),
            logs: HashMap::new(),
            annotations: HashMap::new(),
            log_retries: HashMap::new(),
            cursor: None,
            selection: None,
            scroll_to_error: None,
        }
    }

    pub(super) fn rows(&self) -> Vec<DetailRow> {
        rows::detail_rows(DetailSource {
            jobs: &self.jobs,
            expansion: &self.expansion,
            logs: &self.logs,
            annotations: &self.annotations,
        })
    }
}

impl CiView {
    pub(crate) fn select_run(&mut self, run_id: u64, by_user: bool, cx: &mut Context<Self>) {
        self.user_selected |= by_user;
        if self.selected_run == Some(run_id) {
            return;
        }
        self.selected_run = Some(run_id);
        self.detail = Some(Detail::new(run_id));
        if let Some(ix) = self.shown_runs().iter().position(|r| r.id == run_id) {
            self.runs_scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
        }
        let delay = if by_user { SELECT_DEBOUNCE } else { Duration::ZERO };
        self.load_jobs(run_id, delay, cx);
        cx.notify();
    }

    pub(super) fn load_jobs(&mut self, run_id: u64, delay: Duration, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else { return };
        self.jobs_task = Some(cx.spawn(async move |this, cx| {
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            let result = cx.background_spawn(async move { client.jobs(run_id) }).await;
            this.update(cx, |view, cx| {
                match result {
                    Ok(fetched) => view.jobs_loaded(run_id, fetched.value, cx),
                    Err(err) => {
                        if let Some(detail) = view.detail.as_mut().filter(|d| d.run_id == run_id) {
                            detail.error = Some(err.to_string());
                            detail.loaded = true;
                        }
                    }
                }
                view.schedule(cx);
                cx.notify();
            })
            .ok();
        }));
    }

    pub(super) fn jobs_loaded(&mut self, run_id: u64, jobs: Vec<Job>, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.as_mut().filter(|d| d.run_id == run_id) else {
            return;
        };
        detail.loaded = true;
        detail.error = None;
        for job in &jobs {
            if detail.unfolded.insert((job.id, job.state())) {
                rows::default_expansion(std::slice::from_ref(job), &mut detail.expansion);
                if job.state() == State::Failure && detail.scroll_to_error.is_none() {
                    detail.scroll_to_error = Some(job.id);
                }
            }
        }
        // A finished job's log may differ from the one fetched while it was re-run.
        for job in &jobs {
            let was_active = detail
                .jobs
                .iter()
                .any(|old| old.id == job.id && old.state().is_active());
            if was_active && !job.state().is_active() {
                detail.logs.remove(&job.id);
            }
        }
        detail.jobs = jobs;
        if detail.cursor.is_none() {
            detail.cursor = first_failed_step_row(detail);
        }
        self.ensure_logs(cx);
    }

    /// Fetches logs (and annotations) of finished jobs that have an unfolded step.
    pub(super) fn ensure_logs(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else { return };
        let Some(detail) = self.detail.as_mut() else {
            return;
        };
        let wanted: Vec<(u64, Vec<ci::Step>)> = detail
            .jobs
            .iter()
            .filter(|j| !j.state().is_active() && !detail.logs.contains_key(&j.id))
            .filter(|j| detail.expansion.steps.iter().any(|(id, _)| *id == j.id))
            .map(|j| (j.id, j.steps.clone()))
            .collect();
        for (job_id, steps) in wanted {
            detail.logs.insert(job_id, LogState::Loading);
            let run_id = detail.run_id;
            let client = client.clone();
            cx.spawn(async move |this, cx| {
                let (log, annotations) = cx
                    .background_spawn(async move {
                        let log = client.job_log(job_id).map(|raw| ci::parse_log(&raw, &steps));
                        (log, client.annotations(job_id).unwrap_or_default())
                    })
                    .await;
                this.update(cx, |view, cx| {
                    view.log_loaded(run_id, job_id, log, annotations, cx);
                })
                .ok();
            })
            .detach();
        }
    }

    pub(super) fn log_loaded(
        &mut self,
        run_id: u64,
        job_id: u64,
        log: Result<ci::ParsedLog, ApiError>,
        annotations: Vec<ci::Annotation>,
        cx: &mut Context<Self>,
    ) {
        let Some(detail) = self.detail.as_mut().filter(|d| d.run_id == run_id) else {
            return;
        };
        let retries = detail.log_retries.entry(job_id).or_insert(0);
        if matches!(log, Err(ApiError::NotFound)) && *retries < LOG_RETRIES {
            *retries += 1;
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(LOG_RETRY_DELAY).await;
                this.update(cx, |view, cx| {
                    if let Some(detail) = view.detail.as_mut().filter(|d| d.run_id == run_id) {
                        detail.logs.remove(&job_id);
                    }
                    view.ensure_logs(cx);
                })
                .ok();
            })
            .detach();
            return;
        }
        let state = match log {
            Ok(log) => LogState::Ready(Arc::new(log)),
            Err(ApiError::NotFound) => {
                LogState::Failed("The log is not available (expired or not written yet).".into())
            }
            Err(err) => LogState::Failed(err.to_string()),
        };
        detail.logs.insert(job_id, state);
        if !annotations.is_empty() {
            detail.annotations.insert(job_id, annotations);
        }
        if detail.scroll_to_error == Some(job_id) {
            detail.scroll_to_error = None;
            let rows = detail.rows();
            let target = first_error_row(detail, &rows, job_id);
            if let Some(ix) = target {
                self.detail_scroll.scroll_to_item(ix, ScrollStrategy::Center);
            }
        }
        cx.notify();
    }

    pub(crate) fn toggle_job(&mut self, job_ix: usize, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.as_mut() else {
            return;
        };
        let Some(job) = detail.jobs.get(job_ix) else {
            return;
        };
        if !detail.expansion.jobs.remove(&job.id) {
            detail.expansion.jobs.insert(job.id);
        }
        cx.notify();
    }

    pub(crate) fn toggle_step(&mut self, job_ix: usize, step_ix: usize, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.as_mut() else {
            return;
        };
        let Some(job) = detail.jobs.get(job_ix) else {
            return;
        };
        let Some(step) = job.steps.get(step_ix) else {
            return;
        };
        let key = (job.id, step.number);
        if !detail.expansion.steps.remove(&key) {
            detail.expansion.steps.insert(key);
            // Unfolding again retries a log that failed to load.
            if matches!(detail.logs.get(&job.id), Some(LogState::Failed(_))) {
                detail.logs.remove(&job.id);
                detail.log_retries.remove(&job.id);
            }
        }
        self.ensure_logs(cx);
        cx.notify();
    }

    pub(crate) fn set_cursor(&mut self, row: usize, cx: &mut Context<Self>) {
        if let Some(detail) = self.detail.as_mut() {
            detail.cursor = Some(row);
        }
        self.pane = Pane::Detail;
        cx.notify();
    }

    /// Click on a log line selects it; shift-click extends the selection within the job.
    pub(crate) fn click_log_line(&mut self, job: usize, line: usize, extend: bool, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.as_mut() else {
            return;
        };
        detail.selection = match detail.selection {
            Some((j, anchor, _)) if extend && j == job => Some((job, anchor, line)),
            _ => Some((job, line, line)),
        };
        self.pane = Pane::Detail;
        cx.notify();
    }

    pub(super) fn step_match(&mut self, forward: bool, cx: &mut Context<Self>) {
        let count = self.search_matches(cx).len();
        if count == 0 {
            return;
        }
        self.search_ix = if forward {
            (self.search_ix + 1) % count
        } else {
            (self.search_ix + count - 1) % count
        };
        self.scroll_to_match(cx);
        cx.notify();
    }

    pub(super) fn scroll_to_match(&mut self, cx: &mut Context<Self>) {
        let matches = self.search_matches(cx);
        if let Some(&row) = matches.get(self.search_ix.min(matches.len().saturating_sub(1))) {
            self.detail_scroll.scroll_to_item(row, ScrollStrategy::Center);
        }
    }

    pub(crate) fn search_matches(&self, cx: &App) -> Vec<usize> {
        let Some(detail) = &self.detail else {
            return Vec::new();
        };
        let query = self.search.read(cx).value();
        rows::search_matches(&detail.rows(), &detail.jobs, &detail.logs, &query)
    }

    // Keyboard.
}

pub(super) fn first_failed_step_row(detail: &Detail) -> Option<usize> {
    let rows = detail.rows();
    rows.iter().position(|row| {
        let DetailRow::Step { job, step } = row else {
            return false;
        };
        detail
            .jobs
            .get(*job)
            .and_then(|j| j.steps.get(*step))
            .is_some_and(|s| s.state() == State::Failure)
    })
}

/// Row of the first error line of `job_id`'s failed step (or of its log).
pub(super) fn first_error_row(detail: &Detail, rows: &[DetailRow], job_id: u64) -> Option<usize> {
    let job_ix = detail.jobs.iter().position(|j| j.id == job_id)?;
    let LogState::Ready(log) = detail.logs.get(&job_id)? else {
        return None;
    };
    let range = detail
        .jobs
        .get(job_ix)?
        .failed_step()
        .and_then(|s| rows::step_lines(log, s.number));
    let line = log.first_error(range)?;
    rows.iter()
        .position(|r| *r == DetailRow::Log { job: job_ix, line })
}

pub(super) fn log_text(detail: &Detail, job: usize, lines: std::ops::Range<usize>) -> Option<String> {
    let LogState::Ready(log) = detail.logs.get(&detail.jobs.get(job)?.id)? else {
        return None;
    };
    let text: Vec<&str> = log.lines.get(lines)?.iter().map(|l| l.text.as_str()).collect();
    Some(text.join("\n"))
}

pub(super) fn step_text(detail: &Detail, job: usize, step: usize) -> Option<String> {
    let j = detail.jobs.get(job)?;
    let LogState::Ready(log) = detail.logs.get(&j.id)? else {
        return None;
    };
    let range = rows::step_lines(log, j.steps.get(step)?.number)?;
    log_text(detail, job, range)
}
