//! Closing a project saves its modified files first, behind a progress dialog.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui_kit::component::progress::Progress;
use gpui_kit::component::{ActiveTheme as _, WindowExt as _, v_flex};
use gpui_kit::*;

use super::Workspace;
use crate::editor::CenterTab;

/// Fast saves would only flash the dialog; keep it up long enough to be read.
const MIN_VISIBLE: Duration = Duration::from_millis(350);

/// One modified file, captured when saving starts.
struct PendingFile {
    rel: String,
    path: PathBuf,
    text: SharedString,
    /// Remote tabs also go to the server (after the conflict check).
    upload: Option<super::remote::RemoteUpload>,
}

/// Content of the "Saving files" dialog.
struct SaveProgress {
    done: usize,
    total: usize,
    current: String,
}

impl Render for SaveProgress {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let percent = progress_percent(self.done, self.total);
        v_flex()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .truncate()
                    .child(self.current.clone()),
            )
            .child(Progress::new("save-on-close").value(percent))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("{} of {}", self.done, self.total)),
            )
    }
}

/// Share of saved files in percent (0–100), as `Progress` expects.
fn progress_percent(done: usize, total: usize) -> f32 {
    if total == 0 {
        return 100.;
    }
    done.min(total) as f32 * 100. / total as f32
}

impl Workspace {
    /// Number of editor tabs with unsaved changes.
    pub(super) fn dirty_count(&self, cx: &App) -> usize {
        self.tabs.iter().filter(|t| t.is_dirty(cx)).count()
    }

    /// Writes every modified file off the UI thread while a progress dialog is shown,
    /// then closes the window. A failed write keeps the project open and reports it.
    pub(super) fn save_and_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving_on_close {
            return;
        }
        let files: Vec<PendingFile> = self
            .tabs
            .iter()
            .filter_map(CenterTab::editor)
            .filter(|tab| tab.is_dirty(cx))
            .map(|tab| PendingFile {
                rel: tab.rel.clone(),
                path: tab.path.clone(),
                text: tab.state.read(cx).value(),
                upload: self.remote_upload_for(tab),
            })
            .collect();
        let Some(first) = files.first().map(|f| f.rel.clone()) else {
            return window.remove_window();
        };
        self.saving_on_close = true;
        let progress = cx.new(|_| SaveProgress {
            done: 0,
            total: files.len(),
            current: first,
        });
        let content = progress.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .w(px(420.))
                .title("Saving files…")
                .close_button(false)
                .overlay_closable(false)
                .keyboard(false)
                .child(content.clone())
        });

        cx.spawn_in(window, async move |this, cx| {
            let started = Instant::now();
            for mut file in files {
                progress.update(cx, |p, cx| {
                    p.current.clone_from(&file.rel);
                    cx.notify();
                });
                let (path, bytes, upload) = (file.path.clone(), file.text.clone(), file.upload.take());
                let written = cx
                    .background_spawn(async move {
                        std::fs::write(&path, bytes.as_bytes())?;
                        upload
                            .map(|u| u.run(bytes.as_bytes()))
                            .transpose()
                            .map(Option::flatten)
                    })
                    .await;
                if let Err(err) = written {
                    this.update_in(cx, |ws, window, cx| {
                        ws.saving_on_close = false;
                        window.close_dialog(cx);
                        ws.refresh_git(cx);
                        ws.notify_error(format!("Cannot save {}: {err:#}", file.rel), window, cx);
                    })
                    .ok();
                    return;
                }
                this.update(cx, |ws, cx| ws.mark_saved(&file, cx)).ok();
                progress.update(cx, |p, cx| {
                    p.done += 1;
                    cx.notify();
                });
            }
            if let Some(rest) = MIN_VISIBLE.checked_sub(started.elapsed()) {
                cx.background_executor().timer(rest).await;
            }
            this.update_in(cx, |ws, window, cx| {
                ws.saving_on_close = false;
                window.close_dialog(cx);
                window.remove_window();
            })
            .ok();
        })
        .detach();
    }

    /// Records a background write in the matching tab (found by path: tabs may move meanwhile).
    fn mark_saved(&mut self, file: &PendingFile, cx: &mut Context<Self>) {
        let Some(tab) = self
            .tabs
            .iter_mut()
            .filter_map(|tab| match tab {
                CenterTab::Editor(tab) => Some(tab),
                _ => None,
            })
            .find(|tab| tab.path == file.path)
        else {
            return;
        };
        tab.saved_text = file.text.clone();
        Self::lsp_did_save(tab);
        self.send_extension_event("file_saved", &file.rel, cx);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::progress_percent;

    #[test]
    fn percent_of_saved_files() {
        assert_eq!(progress_percent(0, 4), 0.);
        assert_eq!(progress_percent(1, 4), 25.);
        assert_eq!(progress_percent(4, 4), 100.);
        assert_eq!(progress_percent(5, 4), 100.);
        assert_eq!(progress_percent(0, 0), 100.);
    }
}
