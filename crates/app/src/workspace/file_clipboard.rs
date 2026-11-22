//! Copy / Cut / Paste in the project tree: the clipboard of files waiting to
//! be pasted, and pasting them (off the UI thread) into a folder.

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;

use super::Workspace;
use crate::file_ops::{self, ClipMode, FileClipboard};

impl Workspace {
    pub(super) fn clip_file(
        &mut self,
        rel: String,
        mode: ClipMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = rel.rsplit('/').next().unwrap_or(&rel).to_string();
        let verb = if mode == ClipMode::Cut { "Cut" } else { "Copied" };
        self.file_clipboard = Some(FileClipboard {
            paths: vec![rel],
            mode,
        });
        window.push_notification(
            Notification::info(format!("{verb} {name} — Paste it into a folder")),
            cx,
        );
    }

    pub(super) fn can_paste_files(&self) -> bool {
        self.file_clipboard.is_some()
    }

    /// Pastes the clipboard into `dir` (relative to the root, `""` for the root).
    pub(super) fn paste_files(&mut self, dir: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(clipboard) = self.file_clipboard.clone() else {
            return;
        };
        let root = self.root.clone();
        cx.spawn_in(window, async move |this, cx| {
            let target_dir = root.join(&dir);
            let results = cx
                .background_spawn({
                    let root = root.clone();
                    async move {
                        clipboard
                            .paths
                            .iter()
                            .map(|rel| {
                                let to = file_ops::paste(&root.join(rel), &target_dir, clipboard.mode)?;
                                let to_rel = to
                                    .strip_prefix(&root)
                                    .unwrap_or(&to)
                                    .to_string_lossy()
                                    .into_owned();
                                anyhow::Ok((rel.clone(), to_rel))
                            })
                            .collect::<Vec<_>>()
                    }
                })
                .await;
            this.update_in(cx, |ws, window, cx| {
                let mut pasted = 0;
                for result in results {
                    match result {
                        Ok((from, to)) => {
                            if clipboard.mode == ClipMode::Cut {
                                ws.retarget_tabs(&from, &to);
                            }
                            pasted += 1;
                        }
                        Err(err) => window.push_notification(
                            Notification::error(format!("{err:#}")).title("Paste failed"),
                            cx,
                        ),
                    }
                }
                // A cut is pasted once; copies can be pasted again.
                if clipboard.mode == ClipMode::Cut && pasted > 0 {
                    ws.file_clipboard = None;
                }
                if !dir.is_empty() {
                    ws.expanded.insert(dir.clone());
                }
                ws.after_fs_change(cx);
            })
            .ok();
        })
        .detach();
    }
}
