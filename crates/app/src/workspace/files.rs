//! File operations behind the project tree and tab context menus.

use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context as _, Result, bail};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, WindowExt as _, h_flex};
use gpui_kit::*;

use super::Workspace;
use crate::actions::{FileAction, FileOp};
use crate::editor::CenterTab;

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
fn validate_name(name: &str, allow_nested: bool) -> Result<()> {
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
    pub(super) fn file_context_menu(&self, menu: PopupMenu, rel: &str, is_dir: bool) -> PopupMenu {
        let has_changes = self
            .git
            .as_ref()
            .is_some_and(|g| g.kind_for(rel, is_dir).is_some());
        let action = |op| FileAction::boxed(op, rel.to_string());
        let menu = menu
            .action_context(self.focus_handle.clone())
            .menu("New File…", action(FileOp::NewFile))
            .menu("New Directory…", action(FileOp::NewDirectory))
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
        let abs = self.root.join(&rel);
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
            FileOp::Delete => self.confirm_delete(rel, window, cx),
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
            let tab = &self.tabs[ix];
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
    fn retarget_tabs(&mut self, from: &str, to: &str) {
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

    fn after_fs_change(&mut self, cx: &mut Context<Self>) {
        self.refresh_tree(cx);
        self.reindex(cx);
        self.refresh_git(cx);
    }

    fn confirm_delete(&mut self, rel: String, window: &mut Window, cx: &mut Context<Self>) {
        let workspace = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let (workspace, rel) = (workspace.clone(), rel.clone());
            dialog
                .title(format!("Delete {rel}?"))
                .child("It will be moved to the Trash.")
                .on_ok(move |_, window, cx| {
                    workspace
                        .update(cx, |ws, cx| {
                            let abs = ws.root.join(&rel);
                            match move_to_trash(&abs) {
                                Ok(()) => {
                                    let doomed = |t: &str| t == rel || t.starts_with(&format!("{rel}/"));
                                    ws.close_clean_tabs(|t| t.is_some_and(doomed), window, cx);
                                    ws.after_fs_change(cx);
                                }
                                Err(err) => ws.notify_error(format!("{err:#}"), window, cx),
                            }
                        })
                        .ok();
                    true
                })
        });
    }

    /// Asks for a name; `apply` runs on Enter/OK and its error is shown inline.
    pub(super) fn prompt_name(
        &mut self,
        title: &'static str,
        initial: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
        apply: impl Fn(&mut Workspace, String, &mut Window, &mut Context<Workspace>) -> Result<()> + 'static,
    ) {
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
                    Ok(Ok(())) => window.close_dialog(cx),
                    Ok(Err(err)) => error.update(cx, |e, cx| {
                        *e = Some(format!("{err:#}"));
                        cx.notify();
                    }),
                    Err(_) => window.close_dialog(cx),
                }
            })
        };
        let on_enter = submit.clone();
        cx.subscribe_in(&input, window, move |_, _, event: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { .. } = event {
                on_enter(window, cx);
            }
        })
        .detach();

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
                .title(title)
                .child(Input::new(&input))
                .children(message.map(|m| div().pt_2().text_sm().text_color(danger).child(m)))
                .footer(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .justify_end()
                        .child(
                            Button::new("cancel")
                                .ghost()
                                .small()
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("ok")
                                .primary()
                                .small()
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
