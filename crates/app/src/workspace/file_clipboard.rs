//! Copy / Cut / Paste in the project tree: the clipboard of files waiting to
//! be pasted, and pasting them (or an image from the system clipboard) into a folder.

use std::path::{Path, PathBuf};

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;

use super::Workspace;
use crate::file_ops::{self, ClipMode, FileClipboard, Placement};
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

    /// Pastes into `dir` (relative to the root, `""` for the root). Names already taken
    /// in `dir` are asked for afterwards, one file at a time.
    pub(super) fn paste_files(&mut self, dir: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.paste_source(cx) else {
            return;
        };
        let jobs = match source {
            PasteSource::Image { ext, bytes } => return self.paste_image(dir, ext, bytes, window, cx),
            PasteSource::Files { sources, mode } => PasteJobs {
                files: sources.into_iter().map(|from| (from, None)).collect(),
                mode,
            },
        };
        self.run_paste(dir, jobs, window, cx);
    }

    fn paste_image(
        &mut self,
        dir: String,
        ext: &'static str,
        bytes: Vec<u8>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let root = self.root.clone();
        cx.spawn_in(window, async move |this, cx| {
            let target_dir = root.join(&dir);
            let saved = cx
                .background_spawn(async move { file_ops::save_image(&target_dir, ext, &bytes) })
                .await
                .map(|to| (None, rel_to(&root, &to)));
            this.update_in(cx, |ws, window, cx| {
                ws.finish_paste(&dir, ClipMode::Copy, vec![saved], window, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Copies or moves files off the UI thread, then asks for new names of the taken ones.
    fn run_paste(&mut self, dir: String, jobs: PasteJobs, window: &mut Window, cx: &mut Context<Self>) {
        let root = self.root.clone();
        let mode = jobs.mode;
        cx.spawn_in(window, async move |this, cx| {
            let target_dir = root.join(&dir);
            let (results, conflicts) = cx
                .background_spawn({
                    let root = root.clone();
                    async move { paste_jobs(&root, &target_dir, jobs) }
                })
                .await;
            this.update_in(cx, |ws, window, cx| {
                ws.finish_paste(&dir, mode, results, window, cx);
                ws.ask_paste_names(dir, mode, conflicts, window, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Asks for a new name for the first taken one; the next is asked once this one is answered.
    /// Cancel skips the remaining files.
    fn ask_paste_names(
        &mut self,
        dir: String,
        mode: ClipMode,
        mut conflicts: Vec<Conflict>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if conflicts.is_empty() {
            return;
        }
        let conflict = conflicts.remove(0);
        let title = format!("“{}” already exists here — paste as", conflict.name);
        self.prompt_name(
            title,
            &conflict.suggested,
            window,
            cx,
            move |ws, name, window, cx| {
                super::files::validate_name(&name, false)?;
                let target = ws.root.join(&dir).join(name.trim());
                if target.exists() {
                    anyhow::bail!("{} already exists", name.trim());
                }
                let jobs = PasteJobs {
                    files: vec![(conflict.source.clone(), Some(target))],
                    mode,
                };
                let (dir, rest) = (dir.clone(), conflicts.clone());
                // The name dialog closes after this returns; paste and ask about the rest then.
                let workspace = cx.entity().downgrade();
                window.defer(cx, move |window, cx| {
                    workspace
                        .update(cx, |ws, cx| {
                            ws.run_paste(dir.clone(), jobs, window, cx);
                            ws.ask_paste_names(dir, mode, rest, window, cx);
                        })
                        .ok();
                });
                Ok(())
            },
        );
    }

    /// Reports failures, selects what was pasted and refreshes the tree.
    fn finish_paste(
        &mut self,
        dir: &str,
        mode: ClipMode,
        results: PasteResults,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if results.is_empty() {
            return;
        }
        let mut pasted = Vec::new();
        for result in results {
            match result {
                Ok((from, to)) => {
                    if let (ClipMode::Cut, Some(from)) = (mode, from) {
                        self.retarget_tabs(&from, &to);
                    }
                    pasted.push(to);
                }
                Err(err) => window
                    .push_notification(Notification::error(format!("{err:#}")).title("Paste failed"), cx),
            }
        }
        // A cut is pasted once; copies can be pasted again.
        if mode == ClipMode::Cut && !pasted.is_empty() {
            self.file_clipboard = None;
        }
        if !pasted.is_empty() {
            self.tree_selection.set(pasted);
        }
        if !dir.is_empty() {
            self.expanded.insert(dir.to_string());
        }
        self.after_fs_change(cx);
    }
}

/// Files to paste: each with the target the user chose, or `None` to keep its name.
struct PasteJobs {
    files: Vec<(PathBuf, Option<PathBuf>)>,
    mode: ClipMode,
}

/// A file whose name is already taken in the target folder.
#[derive(Clone)]
struct Conflict {
    source: PathBuf,
    name: String,
    suggested: String,
}

type PasteResults = Vec<Result<(Option<String>, String)>>;

/// Runs off the UI thread: pastes what can be pasted, returns the taken names.
fn paste_jobs(root: &Path, target_dir: &Path, jobs: PasteJobs) -> (PasteResults, Vec<Conflict>) {
    let mut results = Vec::new();
    let mut conflicts = Vec::new();
    for (from, chosen) in jobs.files {
        let target = match chosen {
            Some(target) => Ok(target),
            None => match file_ops::placement(&from, target_dir, jobs.mode) {
                Ok(Placement::Free(target)) => Ok(target),
                Ok(Placement::Taken { name, suggested }) => {
                    conflicts.push(Conflict {
                        source: from,
                        name,
                        suggested,
                    });
                    continue;
                }
                Err(err) => Err(err),
            },
        };
        let pasted = target.and_then(|to| {
            file_ops::paste_to(&from, &to, jobs.mode)?;
            Ok((Some(rel_to(root, &from)), rel_to(root, &to)))
        });
        results.push(pasted);
    }
    (results, conflicts)
}

/// `path` relative to the project root (files from Finder stay absolute).
fn rel_to(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

enum PasteSource {
    Image { ext: &'static str, bytes: Vec<u8> },
    Files { sources: Vec<PathBuf>, mode: ClipMode },
}
