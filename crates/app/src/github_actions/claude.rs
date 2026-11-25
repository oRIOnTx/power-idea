//! "Send to Claude" and "Copy for Claude": a failed job as a compact, redacted Markdown block
//! (`ci::context`), pasted into the project's Claude tab or put on the clipboard.

use std::sync::Arc;

use ci::{ParsedLog, State};
use gpui_kit::*;

use super::rows::{DetailRow, LogState};
use super::{CiEvent, CiView};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Deliver {
    Claude,
    Copy,
}

/// A failed job, and the step to explain (`None`: its first failed step).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    pub job: usize,
    pub step: Option<usize>,
}

impl CiView {
    /// The failed job (or step) under the cursor, else the run's first failed job.
    pub(crate) fn failure_target(&self) -> Option<Target> {
        let detail = self.detail.as_ref()?;
        let failed = |job: usize| detail.jobs.get(job).is_some_and(|j| j.state() == State::Failure);
        let at_cursor = detail.cursor.and_then(|ix| detail.rows().get(ix).cloned());
        let from_cursor = match at_cursor {
            Some(DetailRow::Step { job, step }) if failed(job) => {
                let step_failed = detail
                    .jobs
                    .get(job)
                    .and_then(|j| j.steps.get(step))
                    .is_some_and(|s| s.state() == State::Failure);
                Some(Target {
                    job,
                    step: step_failed.then_some(step),
                })
            }
            Some(row) if failed(row.job()) => Some(Target {
                job: row.job(),
                step: None,
            }),
            _ => None,
        };
        from_cursor.or_else(|| {
            let job = detail.jobs.iter().position(|j| j.state() == State::Failure)?;
            Some(Target { job, step: None })
        })
    }

    /// Builds the failure context (fetching the job log first when needed) and delivers it.
    pub(crate) fn send_failure(
        &mut self,
        target: Option<Target>,
        deliver: Deliver,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = target.or_else(|| self.failure_target()) else {
            return;
        };
        let (Some(client), Some(run), Some(detail)) = (
            self.client.clone(),
            self.selected_run().cloned(),
            self.detail.as_ref(),
        ) else {
            return;
        };
        let Some(job) = detail.jobs.get(target.job).cloned() else {
            return;
        };
        let step = target.step.and_then(|s| job.steps.get(s).cloned());
        let log: Option<Arc<ParsedLog>> = match detail.logs.get(&job.id) {
            Some(LogState::Ready(log)) => Some(log.clone()),
            _ => None,
        };
        let annotations = detail.annotations.get(&job.id).cloned();
        let limit = self.context_limit;
        cx.spawn_in(window, async move |this, cx| {
            let text = cx
                .background_spawn(async move {
                    // Not loaded in the tab yet (the step was never unfolded): fetch it now.
                    let log: Option<Arc<ParsedLog>> = log.or_else(|| {
                        let raw = client.job_log(job.id).ok()?;
                        Some(Arc::new(ci::parse_log(&raw, &job.steps)))
                    });
                    let annotations =
                        annotations.unwrap_or_else(|| client.annotations(job.id).unwrap_or_default());
                    ci::context::build(&ci::context::Failure {
                        repo: client.repo(),
                        run: &run,
                        job: &job,
                        step: step.as_ref(),
                        annotations: &annotations,
                        log: log.as_deref(),
                        limit,
                    })
                })
                .await;
            this.update_in(cx, |view, window, cx| match deliver {
                Deliver::Copy => {
                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                    cx.emit(CiEvent::Notify {
                        title: "CI failure copied".into(),
                        body: format!("{} characters of Markdown for Claude", text.chars().count()),
                        failed: false,
                    });
                }
                Deliver::Claude => {
                    let workspace = view.workspace.clone();
                    // Starting or focusing the Claude tab updates the workspace: not from inside us.
                    window.defer(cx, move |window, cx| {
                        workspace
                            .update(cx, |ws, cx| ws.ci_send_to_claude(text, window, cx))
                            .ok();
                    });
                }
            })
            .ok();
        })
        .detach();
    }
}
