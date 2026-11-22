//! Copy / Cut / Paste in the project tree: the clipboard of files waiting to
//! be pasted, and pasting them (or an image from the system clipboard) into a folder.

use std::path::PathBuf;

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
        // Text on the system clipboard replaces an older screenshot there, so ⌘V pastes these
        // files rather than the image; as in JetBrains IDEs, it also pastes their paths as text.
        let text = paths
            .iter()
            .map(|rel| self.root.join(rel).display().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        cx.write_to_clipboard(ClipboardItem::new_string(text));
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

    pub(super) fn can_paste_files(&self, cx: &App) -> bool {
        self.paste_source(cx).is_some()
    }

    /// What ⌘V pastes: an image (a screenshot) or files copied in Finder from the system
    /// clipboard, otherwise the files copied in the tree.
    fn paste_source(&self, cx: &App) -> Option<PasteSource> {
        let item = cx.read_from_clipboard();
        let entries = item.as_ref().map_or(&[][..], |item| item.entries());
        for entry in entries {
            match entry {
                ClipboardEntry::Image(image) => {
                    return Some(PasteSource::Image {
                        ext: image.format.extension(),
                        bytes: image.bytes.clone(),
                    });
                }
                ClipboardEntry::ExternalPaths(paths) if !paths.paths().is_empty() => {
                    return Some(PasteSource::Files {
                        sources: paths.paths().to_vec(),
                        mode: ClipMode::Copy,
                    });
                }
                _ => {}
            }
        }
        self.file_clipboard.as_ref().map(|clipboard| PasteSource::Files {
            sources: clipboard.paths.iter().map(|rel| self.root.join(rel)).collect(),
            mode: clipboard.mode,
        })
    }

    /// Pastes into `dir` (relative to the root, `""` for the root).
    pub(super) fn paste_files(&mut self, dir: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.paste_source(cx) else {
            return;
        };
        let mode = match &source {
            PasteSource::Files { mode, .. } => *mode,
            PasteSource::Image { .. } => ClipMode::Copy,
        };
        let root = self.root.clone();
        cx.spawn_in(window, async move |this, cx| {
            let target_dir = root.join(&dir);
            let results = cx
                .background_spawn({
                    let root = root.clone();
                    async move {
                        let rel = |path: &std::path::Path| {
                            path.strip_prefix(&root)
                                .unwrap_or(path)
                                .to_string_lossy()
                                .into_owned()
                        };
                        match source {
                            PasteSource::Image { ext, bytes } => {
                                vec![
                                    file_ops::save_image(&target_dir, ext, &bytes).map(|to| (None, rel(&to))),
                                ]
                            }
                            PasteSource::Files { sources, mode } => sources
                                .iter()
                                .map(|from| {
                                    let to = file_ops::paste(from, &target_dir, mode)?;
                                    anyhow::Ok((Some(rel(from)), rel(&to)))
                                })
                                .collect::<Vec<_>>(),
                        }
                    }
                })
                .await;
            this.update_in(cx, |ws, window, cx| {
                let mut pasted = Vec::new();
                for result in results {
                    match result {
                        Ok((from, to)) => {
                            if let (ClipMode::Cut, Some(from)) = (mode, from) {
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
                if mode == ClipMode::Cut && !pasted.is_empty() {
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

enum PasteSource {
    Image { ext: &'static str, bytes: Vec<u8> },
    Files { sources: Vec<PathBuf>, mode: ClipMode },
}
