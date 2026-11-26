//! The transfer queue under the remote tree: uploads and downloads (files or whole
//! folders) with progress, speed, cancel and retry. Each job runs on a background thread.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui_kit::component::progress::Progress;
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use remote::listing::format_size;
use remote::transfer::{self, Direction, SpeedMeter, Status};
use remote::{ErrorKind, RemoteError};

use super::Workspace;
use crate::style;

/// How often running transfers repaint.
const TICK: Duration = Duration::from_millis(250);

#[derive(Clone)]
enum Source {
    Upload { local: PathBuf, remote_dir: String },
    Download { remote: String, local_dir: PathBuf },
}

enum JobState {
    Running,
    Done { bytes: u64, secs: f64 },
    Failed(String),
    Cancelled,
}

pub(super) struct Job {
    id: u64,
    site: String,
    label: String,
    source: Source,
    state: JobState,
    progress: Arc<Mutex<Status>>,
    cancel: Arc<AtomicBool>,
    speed: SpeedMeter,
}

impl Job {
    fn direction(&self) -> Direction {
        match self.source {
            Source::Upload { .. } => Direction::Upload,
            Source::Download { .. } => Direction::Download,
        }
    }

    fn status(&self) -> Status {
        self.progress.lock().map_or_default(|s| s.clone())
    }
}

#[derive(Default)]
pub(in crate::workspace) struct Transfers {
    jobs: Vec<Job>,
    next_id: u64,
    ticking: bool,
}

impl Transfers {
    pub(super) fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    fn running(&self) -> bool {
        self.jobs.iter().any(|j| matches!(j.state, JobState::Running))
    }
}

impl Workspace {
    pub(super) fn queue_upload(
        &mut self,
        site: &str,
        local: PathBuf,
        remote_dir: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = local
            .file_name()
            .map_or_default(|n| n.to_string_lossy().into_owned());
        let source = Source::Upload {
            local,
            remote_dir: remote_dir.to_string(),
        };
        self.add_job(site, format!("↑ {name} → {remote_dir}"), source, window, cx);
    }

    pub(super) fn queue_download(
        &mut self,
        site: &str,
        remote: &str,
        local_dir: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let label = format!("↓ {} → {}", remote::path::file_name(remote), local_dir.display());
        let source = Source::Download {
            remote: remote.to_string(),
            local_dir,
        };
        self.add_job(site, label, source, window, cx);
    }

    fn add_job(
        &mut self,
        site: &str,
        label: String,
        source: Source,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.remote.as_mut() else { return };
        let id = state.transfers.next_id;
        state.transfers.next_id += 1;
        state.transfers.jobs.push(Job {
            id,
            site: site.to_string(),
            label,
            source,
            state: JobState::Running,
            progress: Arc::default(),
            cancel: Arc::default(),
            speed: SpeedMeter::default(),
        });
        self.start_job(id, window, cx);
    }

    fn start_job(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(job) = self
            .remote
            .as_mut()
            .and_then(|s| s.transfers.jobs.iter_mut().find(|j| j.id == id))
        else {
            return;
        };
        job.state = JobState::Running;
        job.cancel.store(false, Ordering::Relaxed);
        job.speed = SpeedMeter::default();
        if let Ok(mut status) = job.progress.lock() {
            *status = Status::default();
        }
        let (source, progress, cancel, site) = (
            job.source.clone(),
            job.progress.clone(),
            job.cancel.clone(),
            job.site.clone(),
        );
        let started = Instant::now();
        self.run_remote_result(
            &site,
            move |conn| {
                let plan = match &source {
                    Source::Upload { local, remote_dir } => transfer::plan_upload(local, remote_dir)?,
                    Source::Download { remote, local_dir } => conn.run(|fs| {
                        let entry = fs.stat(remote)?;
                        transfer::plan_download(fs, &entry, local_dir)
                    })?,
                };
                let direction = match source {
                    Source::Upload { .. } => Direction::Upload,
                    Source::Download { .. } => Direction::Download,
                };
                transfer::execute(conn, direction, &plan, &cancel, &mut |status| {
                    if let Ok(mut shared) = progress.lock() {
                        shared.clone_from(status);
                    }
                })
            },
            move |ws, result, window, cx| ws.finish_job(id, result, started.elapsed(), window, cx),
            window,
            cx,
        );
        self.ensure_transfer_ticker(cx);
        cx.notify();
    }

    fn finish_job(
        &mut self,
        id: u64,
        result: Result<u64, RemoteError>,
        took: Duration,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(job) = self
            .remote
            .as_mut()
            .and_then(|s| s.transfers.jobs.iter_mut().find(|j| j.id == id))
        else {
            return;
        };
        let (site, source) = (job.site.clone(), job.source.clone());
        match result {
            Ok(bytes) => {
                job.state = JobState::Done {
                    bytes,
                    secs: took.as_secs_f64(),
                };
                match source {
                    Source::Upload { remote_dir, .. } => {
                        self.refresh_remote_dir(&site, &remote_dir, window, cx);
                    }
                    // Inside the project the file watcher refreshes the tree.
                    Source::Download { .. } => self.after_fs_change(cx),
                }
            }
            Err(err) if err.kind == ErrorKind::Cancelled && job.cancel.load(Ordering::Relaxed) => {
                job.state = JobState::Cancelled;
            }
            Err(err) => {
                let message = if err.kind == ErrorKind::Cancelled {
                    "Not connected".to_string()
                } else {
                    err.user_message()
                };
                job.state = JobState::Failed(message);
                if err.kind != ErrorKind::Cancelled {
                    self.remote_failed(&site, "Transfer", &err, window, cx);
                }
            }
        }
        cx.notify();
    }

    /// Repaints while transfers run, sampling their speed.
    fn ensure_transfer_ticker(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.remote.as_mut() else { return };
        if state.transfers.ticking {
            return;
        }
        state.transfers.ticking = true;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                let more = this
                    .update(cx, |ws, cx| {
                        let Some(state) = ws.remote.as_mut() else {
                            return false;
                        };
                        let now = Instant::now();
                        for job in &mut state.transfers.jobs {
                            if matches!(job.state, JobState::Running) {
                                let done = job.status().done;
                                job.speed.record(now, done);
                            }
                        }
                        cx.notify();
                        let running = state.transfers.running();
                        state.transfers.ticking = running;
                        running
                    })
                    .unwrap_or(false);
                if !more {
                    break;
                }
            }
        })
        .detach();
    }

    fn cancel_job(&mut self, id: u64) {
        if let Some(job) = self
            .remote
            .as_ref()
            .and_then(|s| s.transfers.jobs.iter().find(|j| j.id == id))
        {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }

    fn remove_job(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(state) = self.remote.as_mut() {
            state.transfers.jobs.retain(|j| j.id != id);
        }
        cx.notify();
    }

    fn clear_finished_jobs(&mut self, cx: &mut Context<Self>) {
        if let Some(state) = self.remote.as_mut() {
            state
                .transfers
                .jobs
                .retain(|j| matches!(j.state, JobState::Running));
        }
        cx.notify();
    }

    pub(super) fn render_transfers(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let state = self.remote.as_ref()?;
        if state.transfers.is_empty() {
            return None;
        }
        let theme = cx.theme();
        let header = h_flex()
            .px_2()
            .py_1()
            .gap_1()
            .border_t_1()
            .border_color(theme.border)
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(
                div()
                    .flex_1()
                    .child(format!("TRANSFERS ({})", state.transfers.jobs.len())),
            )
            .child(
                style::icon_button("remote-transfers-clear", IconName::Delete, "Clear Finished")
                    .on_click(cx.listener(|ws, _, _, cx| ws.clear_finished_jobs(cx))),
            );
        let rows = state
            .transfers
            .jobs
            .iter()
            .rev()
            .map(|job| self.render_job(job, cx));
        Some(
            v_flex()
                .flex_none()
                .max_h(px(180.))
                .child(header)
                .child(div().id("remote-transfers").overflow_y_scroll().children(rows))
                .into_any_element(),
        )
    }

    fn render_job(&self, job: &Job, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let status = job.status();
        let id = job.id;
        let (detail, color) = match &job.state {
            JobState::Running => {
                let speed = job
                    .speed
                    .bytes_per_sec()
                    .map_or_default(|s| format!(" · {}/s", format_size(s)));
                let files = if status.files_total > 1 {
                    format!(" · {}/{} files", status.files_done, status.files_total)
                } else {
                    String::new()
                };
                (
                    format!(
                        "{} of {}{files}{speed}",
                        format_size(status.done),
                        format_size(status.total)
                    ),
                    theme.muted_foreground,
                )
            }
            JobState::Done { bytes, secs } => (
                format!("Done · {} in {:.1} s", format_size(*bytes), secs),
                theme.green,
            ),
            JobState::Failed(err) => (err.clone(), theme.danger),
            JobState::Cancelled => ("Cancelled".into(), theme.muted_foreground),
        };
        let percent = if status.total == 0 {
            if matches!(job.state, JobState::Done { .. }) {
                100.
            } else {
                0.
            }
        } else {
            status.done as f32 * 100. / status.total as f32
        };
        let icon = match job.direction() {
            Direction::Upload => style::lucide("upload"),
            Direction::Download => style::lucide("download"),
        };
        let running = matches!(job.state, JobState::Running);
        let retryable = matches!(job.state, JobState::Failed(_) | JobState::Cancelled);
        v_flex()
            .px_2()
            .py_1()
            .gap_0p5()
            .child(
                h_flex()
                    .gap_1p5()
                    .text_xs()
                    .child(icon.with_size(px(12.)).text_color(theme.muted_foreground))
                    .child(div().flex_1().min_w_0().truncate().child(job.label.clone()))
                    .when(running, |row| {
                        row.child(
                            style::icon_button(("remote-job-cancel", id as usize), IconName::Close, "Cancel")
                                .xsmall()
                                .on_click(cx.listener(move |ws, _, _, _| ws.cancel_job(id))),
                        )
                    })
                    .when(retryable, |row| {
                        row.child(
                            style::icon_button_sized(
                                ("remote-job-retry", id as usize),
                                style::lucide("rotate-ccw"),
                                "Retry",
                                style::BUTTON_SMALL,
                            )
                            .on_click(cx.listener(move |ws, _, window, cx| ws.start_job(id, window, cx))),
                        )
                    })
                    .when(!running, |row| {
                        row.child(
                            style::icon_button(("remote-job-remove", id as usize), IconName::Close, "Remove")
                                .xsmall()
                                .on_click(cx.listener(move |ws, _, _, cx| ws.remove_job(id, cx))),
                        )
                    }),
            )
            .when(running, |el| {
                el.child(Progress::new(("remote-job-progress", id as usize)).value(percent))
            })
            .child(div().text_xs().text_color(color).truncate().child(detail))
            .into_any_element()
    }
}
