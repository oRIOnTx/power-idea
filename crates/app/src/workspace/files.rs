//! File operations behind the project tree and tab context menus.

use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context as _, Result, bail};
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::Workspace;
use crate::actions::{FileAction, FileOp};
use crate::editor::CenterTab;
use crate::file_ops::ClipMode;
use crate::tree_selection::outermost;
use crate::{file_icons, style};

/// Moves `path` into the user's Trash (recoverable), picking a free name.
fn move_to_trash(path: &Path) -> Result<()> {
    let trash = dirs::home_dir().context("no home directory")?.join(".Trash");
    let name = path
        .file_name()
        .context("invalid path")?
        .to_string_lossy()
        .into_owned();
    let mut target = trash.join(&name);
    let mut n = 1;
    while target.exists() {
        n += 1;
        target = trash.join(format!("{name} {n}"));
    }
    fs::rename(path, &target).with_context(|| format!("moving {} to Trash", path.display()))
}

/// Validates a user-entered name; nested paths are allowed only when creating.
pub(super) fn validate_name(name: &str, allow_nested: bool) -> Result<()> {
    let name = name.trim();
    if name.is_empty() {
        bail!("name is empty");
    }
    if name.split('/').any(|part| part == ".." || part == ".") || name.starts_with('/') {
        bail!("name must stay inside the project");
    }
    if !allow_nested && name.contains('/') {
        bail!("name must not contain '/'");
    }
    Ok(())
}

/// Rows listed in the delete confirmation; the rest are summed up.
const DELETE_LIST_MAX: usize = 8;

/// The files about to be deleted: icon, name and folder, at most `DELETE_LIST_MAX` rows.
fn delete_list(items: &[(String, bool)], cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let rows = items.iter().take(DELETE_LIST_MAX).map(|(rel, is_dir)| {
        let name = rel.rsplit('/').next().unwrap_or(rel).to_string();
        let icon = if *is_dir {
            Icon::new(IconName::Folder).text_color(file_icons::folder_tint(&name).color(cx))
        } else {
            file_icons::icon_for_file(&name, cx)
        };
        h_flex()
            .gap_2()
            .min_w_0()
            .child(div().flex_none().child(icon.with_size(style::TREE_ICON)))
            .child(div().flex_none().text_color(theme.foreground).child(name))
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(parent_rel(rel).to_string()),
            )
    });
    let more = items.len().saturating_sub(DELETE_LIST_MAX);
    v_flex()
        .mt_1()
        .gap_1()
        .px_3()
        .py_2()
        .rounded(theme.radius)
        .border_1()
        .border_color(theme.border)
        .bg(theme.muted)
        .text_sm()
        .children(rows)
        .when(more > 0, |list| {
            list.child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format!("and {more} more")),
            )
        })
}

fn join_rel(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

fn parent_rel(rel: &str) -> &str {
    rel.rsplit_once('/').map_or("", |(dir, _)| dir)
}

impl Workspace {
    /// Context menu for a project tree entry (`rel` relative to the root).
    pub(super) fn file_context_menu(&self, menu: PopupMenu, rel: &str, is_dir: bool, cx: &App) -> PopupMenu {
        let has_changes = self
            .git
            .as_ref()
            .is_some_and(|g| g.kind_for(rel, is_dir).is_some());
        let action = |op| FileAction::boxed(op, rel.to_string());
        let menu = menu
            // Focus returns to the project view, so ⌘C / ⌘V keep working after the menu.
            .action_context(self.project_focus.clone())
            .menu("New File…", action(FileOp::NewFile))
            .menu("New Directory…", action(FileOp::NewDirectory))
            .separator()
            .menu("Cut", action(FileOp::Cut))
            .menu("Copy", action(FileOp::CopyFile))
            .menu_with_disabled("Paste", action(FileOp::Paste), !self.can_paste_files(cx))
            .separator()
            .menu("Rename…", action(FileOp::Rename))
            .menu("Delete…", action(FileOp::Delete))
            .separator()
            .menu("Copy Path", action(FileOp::CopyPath))
            .menu("Copy Relative Path", action(FileOp::CopyRelativePath))
            .separator()
            .menu("Reveal in Finder", action(FileOp::RevealInFinder))
            .menu("Open in Terminal", action(FileOp::OpenInTerminal));
        if has_changes && !is_dir {
            menu.separator().menu("Show Diff", action(FileOp::ShowDiff))
        } else {
            menu
        }
    }

    /// Context menu for the project root row: what applies to the project folder itself
    /// (no Cut / Copy / Rename / Delete).
    pub(super) fn root_context_menu(&self, menu: PopupMenu, cx: &App) -> PopupMenu {
        let action = |op| FileAction::boxed(op, "");
        menu.action_context(self.project_focus.clone())
            .menu("New File…", action(FileOp::NewFile))
            .menu("New Directory…", action(FileOp::NewDirectory))
            .separator()
            .menu_with_disabled("Paste", action(FileOp::Paste), !self.can_paste_files(cx))
            .separator()
            .menu("Copy Path", action(FileOp::CopyPath))
            .separator()
            .menu("Reveal in Finder", action(FileOp::RevealInFinder))
            .menu("Open in Terminal", action(FileOp::OpenInTerminal))
    }

    /// Context menu for an editor tab.
    pub(super) fn tab_context_menu(&self, menu: PopupMenu, rel: &str) -> PopupMenu {
        let action = |op| FileAction::boxed(op, rel.to_string());
        menu.action_context(self.focus_handle.clone())
            .menu("Close", action(FileOp::CloseTab))
            .menu("Close Other Tabs", action(FileOp::CloseOtherTabs))
            .menu("Close All Tabs", action(FileOp::CloseAllTabs))
            .separator()
            .menu("Copy Path", action(FileOp::CopyPath))
            .menu("Reveal in Finder", action(FileOp::RevealInFinder))
    }

    pub(super) fn handle_file_action(
        &mut self,
        action: &FileAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let rel = action.path.to_string();
        // `join("")` would add a trailing slash to the root path.
        let abs = if rel.is_empty() {
            self.root.clone()
        } else {
            self.root.join(&rel)
        };
        let dir = if abs.is_dir() {
            rel.clone()
        } else {
            parent_rel(&rel).to_string()
        };
        match action.op {
            FileOp::NewFile => self.prompt_name("New File", "", window, cx, move |ws, name, window, cx| {
                validate_name(&name, true)?;
                let rel = join_rel(&dir, name.trim());
                let path = ws.root.join(&rel);
                if path.exists() {
                    bail!("{rel} already exists");
                }
                fs::create_dir_all(path.parent().context("invalid path")?)?;
                fs::write(&path, "")?;
                ws.after_fs_change(cx);
                ws.open_file(&rel, None, window, cx);
                Ok(())
            }),
            FileOp::NewDirectory => {
                self.prompt_name("New Directory", "", window, cx, move |ws, name, _, cx| {
                    validate_name(&name, true)?;
                    let rel = join_rel(&dir, name.trim());
                    fs::create_dir_all(ws.root.join(&rel))?;
                    ws.expanded.insert(rel);
                    ws.after_fs_change(cx);
                    Ok(())
                });
            }
            FileOp::Rename => {
                let current = rel.rsplit('/').next().unwrap_or(&rel).to_string();
                self.prompt_name("Rename", &current, window, cx, move |ws, name, _, cx| {
                    validate_name(&name, false)?;
                    let target = join_rel(parent_rel(&rel), name.trim());
                    if ws.root.join(&target).exists() {
                        bail!("{target} already exists");
                    }
                    fs::rename(ws.root.join(&rel), ws.root.join(&target))?;
                    ws.retarget_tabs(&rel, &target);
                    ws.after_fs_change(cx);
                    Ok(())
                });
            }
            FileOp::Delete => self.confirm_delete(self.menu_targets(&rel), window, cx),
            FileOp::Cut => self.clip_paths(self.menu_targets(&rel), ClipMode::Cut, window, cx),
            FileOp::CopyFile => self.clip_paths(self.menu_targets(&rel), ClipMode::Copy, window, cx),
            // Into the clicked folder, or the folder of the clicked file.
            FileOp::Paste => self.paste_files(dir, window, cx),
            FileOp::CopyPath | FileOp::CopyRelativePath => {
                let text = if action.op == FileOp::CopyPath {
                    abs.display().to_string()
                } else {
                    rel
                };
                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                window.push_notification(Notification::info(text).title("Copied"), cx);
            }
            FileOp::RevealInFinder => {
                if let Err(err) = Command::new("open").arg("-R").arg(&abs).spawn() {
                    self.notify_error(format!("Cannot reveal {rel}: {err}"), window, cx);
                }
            }
            FileOp::OpenInTerminal => self.new_terminal_in(self.root.join(&dir), window, cx),
            FileOp::ShowDiff => {
                let change = self.git.as_ref().and_then(|g| {
                    let path = g.repo_path(&rel);
                    g.status.changes.iter().find(|c| c.path == path).cloned()
                });
                if let Some(change) = change {
                    self.open_change_diff(change, window, cx);
                }
            }
            FileOp::CloseTab => {
                if let Some(ix) = self.editor_tab_index(&rel) {
                    self.close_tab(ix, window, cx);
                }
            }
            FileOp::CloseOtherTabs => {
                self.close_clean_tabs(|tab_rel| tab_rel != Some(rel.as_str()), window, cx);
            }
            FileOp::CloseAllTabs => self.close_clean_tabs(|_| true, window, cx),
        }
    }

    fn editor_tab_index(&self, rel: &str) -> Option<usize> {
        self.tabs
            .iter()
            .position(|t| t.editor().is_some_and(|e| e.rel == rel))
    }

    /// Closes matching tabs, keeping those with unsaved edits open.
    fn close_clean_tabs(
        &mut self,
        matches: impl Fn(Option<&str>) -> bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut kept_dirty = 0;
        for ix in (0..self.tabs.len()).rev() {
            let Some(tab) = self.tabs.get(ix) else { continue };
            if !matches(tab.editor().map(|e| e.rel.as_str())) {
                continue;
            }
            if tab.is_dirty(cx) {
                kept_dirty += 1;
            } else {
                self.close_tab_now(ix, window, cx);
            }
        }
        if kept_dirty > 0 {
            let message = format!("{kept_dirty} tab(s) with unsaved changes were kept open");
            window.push_notification(Notification::warning(message), cx);
        }
    }

    /// Points open editors at their new location after a rename.
    pub(super) fn retarget_tabs(&mut self, from: &str, to: &str) {
        for tab in &mut self.tabs {
            let CenterTab::Editor(tab) = tab else { continue };
            let new_rel = if tab.rel == from {
                to.to_string()
            } else if let Some(rest) = tab.rel.strip_prefix(&format!("{from}/")) {
                format!("{to}/{rest}")
            } else {
                continue;
            };
            tab.path = self.root.join(&new_rel);
            tab.rel = new_rel;
        }
    }

    pub(super) fn after_fs_change(&mut self, cx: &mut Context<Self>) {
        self.refresh_tree(cx);
        self.reindex(cx);
        self.refresh_git(cx);
    }

    /// ⌦ / ⌫ in the project tree: the selected files and folders.
    pub(super) fn delete_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = self.tree_selection.paths.clone();
        self.confirm_delete(paths, window, cx);
    }

    /// Asks, then moves `paths` (relative to the root) to the Trash.
    fn confirm_delete(&mut self, paths: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        // Deleting a folder takes its children along; never the project root itself.
        let paths: Vec<String> = outermost(&paths).into_iter().filter(|p| !p.is_empty()).collect();
        let title = match paths.as_slice() {
            [] => return,
            [rel] => format!("Delete “{}”?", rel.rsplit('/').next().unwrap_or(rel)),
            _ => format!("Delete {} items?", paths.len()),
        };
        let items: Vec<(String, bool)> = paths
            .iter()
            .map(|rel| (rel.clone(), self.root.join(rel).is_dir()))
            .collect();
        let workspace = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |dialog, _, cx| {
            let (workspace, paths) = (workspace.clone(), paths.clone());
            style::danger_confirm(dialog, "trash", "Move to Trash", cx)
                .width(px(460.))
                .title(title.clone())
                .description("Moved to the Trash: you can put it back from Finder.")
                .child(delete_list(&items, cx))
                .on_ok(move |_, window, cx| {
                    workspace
                        .update(cx, |ws, cx| ws.trash_paths(&paths, window, cx))
                        .ok();
                    true
                })
        });
    }

    fn trash_paths(&mut self, paths: &[String], window: &mut Window, cx: &mut Context<Self>) {
        for rel in paths {
            if let Err(err) = move_to_trash(&self.root.join(rel)) {
                self.notify_error(format!("{err:#}"), window, cx);
                continue;
            }
            let doomed = |t: &str| t == rel || t.starts_with(&format!("{rel}/"));
            self.close_clean_tabs(|t| t.is_some_and(doomed), window, cx);
        }
        self.after_fs_change(cx);
    }

    /// Asks for a name; `apply` runs on Enter/OK and its error is shown inline.
    pub(super) fn prompt_name(
        &mut self,
        title: impl Into<SharedString>,
        initial: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
        apply: impl Fn(&mut Workspace, String, &mut Window, &mut Context<Workspace>) -> Result<()> + 'static,
    ) {
        let title: SharedString = title.into();
        let input = cx.new(|cx| InputState::new(window, cx).default_value(initial.to_string()));
        let error: Entity<Option<String>> = cx.new(|_| None);
        let apply = std::rc::Rc::new(apply);
        let workspace = cx.entity().downgrade();

        let submit = {
            let (input, error, workspace) = (input.clone(), error.clone(), workspace);
            std::rc::Rc::new(move |window: &mut Window, cx: &mut App| {
                let name = input.read(cx).value().to_string();
                let apply = apply.clone();
                let result = workspace.update(cx, |ws, cx| apply(ws, name, window, cx));
                match result {
                    Ok(Err(err)) => error.update(cx, |e, cx| {
                        *e = Some(format!("{err:#}"));
                        cx.notify();
                    }),
                    // Applied, or the workspace is gone: either way the dialog is done.
                    Ok(Ok(())) | Err(_) => window.close_dialog(cx),
                }
            })
        };
        // Not `cx.subscribe_in`: its callback runs inside a Workspace update, and `submit`
        // updates the workspace again (gpui panics → the app aborts).
        let on_enter = submit.clone();
        style::submit_on_enter(&input, window, cx, move |window, cx| on_enter(window, cx));

        let focused = std::cell::Cell::new(false);
        window.open_dialog(cx, move |dialog, window, cx| {
            if !focused.replace(true) {
                input.update(cx, |i, cx| i.focus(window, cx));
            }
            let submit = submit.clone();
            let message = error.read(cx).clone();
            let danger = cx.theme().danger;
            dialog
                .w(px(420.))
                .title(title.clone())
                .child(Input::new(&input))
                .children(message.map(|m| div().pt_2().text_sm().text_color(danger).child(m)))
                .footer(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .justify_end()
                        .child(
                            style::dialog_button("cancel")
                                .ghost()
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            style::dialog_button("ok")
                                .primary()
                                .label("OK")
                                .on_click(move |_, window, cx| submit(window, cx)),
                        ),
                )
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{join_rel, parent_rel, validate_name};

    #[test]
    fn validates_names() {
        assert!(validate_name("main.rs", false).is_ok());
        assert!(validate_name("src/new.rs", true).is_ok());
        assert!(validate_name("src/new.rs", false).is_err());
        assert!(validate_name("../escape", true).is_err());
        assert!(validate_name("/abs", true).is_err());
        assert!(validate_name("  ", true).is_err());
    }

    #[test]
    fn joins_relative_paths() {
        assert_eq!(join_rel("", "a.rs"), "a.rs");
        assert_eq!(join_rel("src", "a.rs"), "src/a.rs");
        assert_eq!(parent_rel("src/a.rs"), "src");
        assert_eq!(parent_rel("a.rs"), "");
    }
}
