//! Keyboard and clipboard in the GitHub Actions tab: moving through runs and detail rows,
//! Enter / Esc, Open on GitHub, copying log lines.

use gpui_kit::*;

use super::detail::{first_failed_step_row, log_text, step_text};
use super::rows::DetailRow;
use super::{CiView, Pane};

impl CiView {
    pub(super) fn move_cursor(&mut self, delta: isize, cx: &mut Context<Self>) {
        match self.pane {
            Pane::Runs => {
                let runs = self.shown_runs();
                if runs.is_empty() {
                    return;
                }
                let current = self
                    .selected_run
                    .and_then(|id| runs.iter().position(|r| r.id == id));
                let next = step_index(current, delta, runs.len());
                if let Some(id) = runs.get(next).map(|r| r.id) {
                    self.select_run(id, true, cx);
                }
            }
            Pane::Detail => {
                let Some(detail) = self.detail.as_mut() else {
                    return;
                };
                let rows = detail.rows();
                let mut ix = detail.cursor.unwrap_or(0) as isize;
                loop {
                    ix += delta.signum();
                    let Some(row) = usize::try_from(ix).ok().and_then(|i| rows.get(i)) else {
                        return;
                    };
                    if matches!(
                        row,
                        DetailRow::Job { .. } | DetailRow::Step { .. } | DetailRow::Annotation { .. }
                    ) {
                        break;
                    }
                }
                let ix = ix as usize;
                detail.cursor = Some(ix);
                self.detail_scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
                cx.notify();
            }
        }
    }

    pub(super) fn open_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.pane {
            Pane::Runs => {
                self.pane = Pane::Detail;
                if let Some(detail) = self.detail.as_mut()
                    && detail.cursor.is_none()
                {
                    detail.cursor = first_failed_step_row(detail).or(Some(0));
                }
                cx.notify();
            }
            Pane::Detail => {
                let Some(detail) = &self.detail else { return };
                let Some(row) = detail.cursor.and_then(|ix| detail.rows().get(ix).cloned()) else {
                    return;
                };
                match row {
                    DetailRow::Job { job } => self.toggle_job(job, cx),
                    DetailRow::Step { job, step } => self.toggle_step(job, step, cx),
                    DetailRow::Annotation { job, ix } => self.open_annotation(job, ix, window, cx),
                    DetailRow::Log { .. } | DetailRow::Note { .. } => {}
                }
            }
        }
    }

    pub(super) fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.read(cx).focus_handle(cx).is_focused(window)
            || !self.search.read(cx).value().is_empty()
        {
            self.search
                .update(cx, |input, cx| input.set_value("", window, cx));
            self.focus.focus(window, cx);
        } else if self.pane == Pane::Detail {
            self.pane = Pane::Runs;
        }
        cx.notify();
    }

    /// The run, or the job (at its step) under the detail cursor.
    pub(crate) fn github_url(&self) -> Option<String> {
        let run = self.selected_run()?;
        if self.pane == Pane::Detail
            && let Some(detail) = &self.detail
            && let Some(row) = detail.cursor.and_then(|ix| detail.rows().get(ix).cloned())
            && let Some(job) = detail.jobs.get(row.job())
            && let Some(url) = &job.html_url
        {
            if let DetailRow::Step { step, .. } = row
                && let Some(step) = job.steps.get(step)
            {
                return Some(format!("{url}#step:{}:1", step.number));
            }
            return Some(url.clone());
        }
        Some(run.html_url.clone())
    }

    pub(crate) fn open_on_github(&mut self, cx: &mut Context<Self>) {
        let url = self.github_url().or_else(|| {
            self.client
                .as_ref()
                .map(|c| format!("{}/actions", c.repo().html_url()))
        });
        if let Some(url) = url {
            cx.open_url(&url);
        }
    }

    pub(crate) fn open_annotation(
        &mut self,
        job: usize,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(annotation) = self
            .detail
            .as_ref()
            .and_then(|d| d.annotations.get(&d.jobs.get(job)?.id)?.get(ix))
        else {
            return;
        };
        let path = self.root.join(&annotation.path);
        let line = annotation.start_line.saturating_sub(1);
        let workspace = self.workspace.clone();
        // The workspace may update this view while opening the file: never from inside our update.
        window.defer(cx, move |window, cx| {
            workspace
                .update(cx, |ws, cx| ws.ci_open_location(&path, line, window, cx))
                .ok();
        });
    }

    pub(crate) fn run_in_terminal(
        &mut self,
        command: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let workspace = self.workspace.clone();
        window.defer(cx, move |window, cx| {
            workspace
                .update(cx, |ws, cx| ws.ci_run_in_terminal(command, window, cx))
                .ok();
        });
    }

    /// The selected log lines, else the step under the cursor, as plain text.
    pub(super) fn copy_selection(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = &self.detail else { return };
        let text = if let Some((job, a, b)) = detail.selection {
            log_text(detail, job, a.min(b)..a.max(b) + 1)
        } else if let Some(DetailRow::Step { job, step }) =
            detail.cursor.and_then(|ix| detail.rows().get(ix).cloned())
        {
            step_text(detail, job, step)
        } else {
            None
        };
        if let Some(text) = text {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    pub(crate) fn copy_step(&mut self, job: usize, step: usize, cx: &mut Context<Self>) {
        if let Some(text) = self.detail.as_ref().and_then(|d| step_text(d, job, step)) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }
}

fn step_index(current: Option<usize>, delta: isize, len: usize) -> usize {
    match current {
        None => 0,
        Some(ix) => (ix as isize + delta).clamp(0, len as isize - 1) as usize,
    }
}
