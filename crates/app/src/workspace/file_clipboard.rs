//! Copy / Cut / Paste in the project tree: the clipboard of files waiting to
//! be pasted, and pasting them (off the UI thread) into a folder.

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;

use super::Workspace;
use crate::file_ops::{self, ClipMode, FileClipboard};
use crate::tree_selection::outermost;

impl Workspace {
    /// Puts `paths` (relative to the root) on the file clipboard.
    pub(super) fn clip_paths(
        &mut self,
        paths: Vec<String>,
        mode: ClipMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let paths = outermost(&paths);
        let what = match paths.as_slice() {
            [] => return,
            [rel] => rel.rsplit('/').next().unwrap_or(rel).to_string(),
            _ => format!("{} items", paths.len()),
        };
        let verb = if mode == ClipMode::Cut { "Cut" } else { "Copied" };
        self.file_clipboard = Some(FileClipboard { paths, mode });
        window.push_notification(
            Notification::info(format!("{verb} {what} — Paste into a folder with ⌘V")),
            cx,
        );
    }

    /// ⌘X / ⌘C in the project tree: the selected files and folders.
    pub(super) fn clip_selection(&mut self, mode: ClipMode, window: &mut Window, cx: &mut Context<Self>) {
        let paths = self.tree_selection.paths.clone();
        self.clip_paths(paths, mode, window, cx);
    }

    /// The files a tree context menu acts on: the whole selection when `rel` is part of it.
    pub(super) fn menu_targets(&self, rel: &str) -> Vec<String> {
        if self.tree_selection.contains(rel) {
            self.tree_selection.paths.clone()
        } else {
            vec![rel.to_string()]
        }
    }

    /// ⌘V in the project tree: into the selected folder, the selected file's folder, or the root.
    pub(super) fn paste_into_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dir = self.tree_selection.paths.first().map_or_else(String::new, |rel| {
            if self.root.join(rel).is_dir() {
                rel.clone()
            } else {
                rel.rsplit_once('/')
                    .map_or_else(String::new, |(dir, _)| dir.to_string())
            }
        });
        self.paste_files(dir, window, cx);
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
                let mut pasted = Vec::new();
                for result in results {
                    match result {
                        Ok((from, to)) => {
                            if clipboard.mode == ClipMode::Cut {
                                ws.retarget_tabs(&from, &to);
                            }
                            pasted.push(to);
                        }
                        Err(err) => window.push_notification(
                            Notification::error(format!("{err:#}")).title("Paste failed"),
                            cx,
                        ),
                    }
                }
                // A cut is pasted once; copies can be pasted again.
                if clipboard.mode == ClipMode::Cut && !pasted.is_empty() {
                    ws.file_clipboard = None;
                }
                if !pasted.is_empty() {
                    ws.tree_selection.set(pasted);
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
