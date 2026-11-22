//! Guards against losing unsaved edits when closing tabs, windows or the app.

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Sizable as _, WindowExt as _, h_flex};
use gpui_kit::*;

use super::{Workspace, Workspaces};

type Choice = Rc<dyn Fn(&mut Workspace, &mut Window, &mut Context<Workspace>)>;

/// Set when the user chose "Don't Save" on quit, so no recovery copy is kept.
#[derive(Default)]
struct DiscardOnQuit(bool);

impl Global for DiscardOnQuit {}

/// Runs while the app terminates (also for Dock → Quit, which cannot be
/// cancelled): saves workspace state and keeps unsaved text for recovery.
pub(crate) fn store_recovery(cx: &mut App) {
    let discard = cx.try_global::<DiscardOnQuit>().is_some_and(|d| d.0);
    let Some(recovery) = crate::recovery::Recovery::new() else {
        return;
    };
    for (_, ws) in Workspaces::live(cx) {
        let (root, files) = ws.update(cx, |ws, cx| {
            ws.save_session_now(cx);
            let files: Vec<(String, String)> = ws
                .tabs
                .iter()
                .filter_map(|tab| tab.editor())
                .filter(|tab| !discard && tab.is_dirty(cx))
                .map(|tab| (tab.rel.clone(), tab.state.read(cx).value().to_string()))
                .collect();
            (ws.root.clone(), files)
        });
        if let Err(err) = recovery.store(&root, &files) {
            eprintln!("keeping unsaved changes of {}: {err:#}", root.display());
        }
    }
}

/// ⌘Q: quits right away, or first asks about unsaved files in any project window.
pub(crate) fn request_quit(cx: &mut App) {
    let live = Workspaces::live(cx);
    let mut dirty = Vec::new();
    for (handle, ws) in &live {
        let count = ws.update(cx, |ws, cx| {
            ws.save_session_now(cx);
            ws.tabs.iter().filter(|t| t.is_dirty(cx)).count()
        });
        if count > 0 {
            dirty.push((*handle, ws.clone(), count));
        }
    }
    let Some((handle, ws, _)) = dirty.first().cloned() else {
        return cx.quit();
    };
    let total: usize = dirty.iter().map(|(_, _, n)| n).sum();
    let title = format!(
        "{total} file{} with unsaved changes",
        if total == 1 { "" } else { "s" }
    );
    let message = match dirty.len() {
        1 => "Save them before quitting?".to_string(),
        projects => format!("They are in {projects} projects. Save them before quitting?"),
    };
    handle
        .update(cx, |_, window, cx| {
            window.activate_window();
            ws.update(cx, |ws, cx| {
                ws.ask_unsaved(
                    title,
                    message,
                    // Saving touches every workspace, including this one, so run it after this update.
                    Rc::new(|_, _, cx| App::defer(cx, save_all_and_quit)),
                    Rc::new(|_, _, cx| {
                        cx.set_global(DiscardOnQuit(true));
                        cx.quit();
                    }),
                    window,
                    cx,
                );
            });
        })
        .ok();
}

fn save_all_and_quit(cx: &mut App) {
    for (handle, ws) in Workspaces::live(cx) {
        let saved = handle
            .update(cx, |_, window, cx| {
                ws.update(cx, |ws, cx| ws.save_all(window, cx))
            })
            .unwrap_or(true);
        // A failed save has already been reported; stay open so nothing is lost.
        if !saved {
            return;
        }
    }
    cx.quit();
}

impl Workspace {
    /// Closes a tab, asking first when it has unsaved changes.
    pub(super) fn close_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(ix) else { return };
        if !tab.is_dirty(cx) {
            return self.close_tab_now(ix, window, cx);
        }
        let title = tab.title();
        self.ask_unsaved(
            format!("Save changes to {title}?"),
            "Your changes will be lost if you don't save them.".into(),
            Rc::new(move |ws, window, cx| {
                if ws.save_tab(ix, window, cx) {
                    ws.close_tab_now(ix, window, cx);
                }
            }),
            Rc::new(move |ws, window, cx| ws.close_tab_now(ix, window, cx)),
            window,
            cx,
        );
    }

    /// Saves every editor tab; returns false if any save failed.
    fn save_all(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        (0..self.tabs.len()).all(|ix| !self.tabs[ix].is_dirty(cx) || self.save_tab(ix, window, cx))
    }

    /// Installs the window-close guard. Called once from `Workspace::new`.
    pub(super) fn guard_window_close(&self, window: &mut Window, cx: &mut Context<Self>) {
        let workspace = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            workspace
                .upgrade()
                .is_none_or(|ws| ws.update(cx, |ws, cx| ws.can_close_window(window, cx)))
        });
    }

    /// Returns true if the window can close now; otherwise asks about unsaved
    /// files and closes the window after the user decides.
    pub(crate) fn can_close_window(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.save_session_now(cx);
        let dirty = self.tabs.iter().filter(|t| t.is_dirty(cx)).count();
        if dirty == 0 {
            return true;
        }
        self.ask_unsaved(
            format!(
                "{dirty} file{} with unsaved changes",
                if dirty == 1 { "" } else { "s" }
            ),
            "Save them before closing the project window?".into(),
            Rc::new(|ws, window, cx| {
                if ws.save_all(window, cx) {
                    window.remove_window();
                }
            }),
            Rc::new(|_, window, _| window.remove_window()),
            window,
            cx,
        );
        false
    }

    /// Save / Don't Save / Cancel dialog.
    fn ask_unsaved(
        &mut self,
        title: String,
        message: String,
        on_save: Choice,
        on_discard: Choice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let workspace = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, _| {
            let choose = |choice: Choice| {
                let workspace = workspace.clone();
                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    window.close_dialog(cx);
                    let choice = choice.clone();
                    workspace.update(cx, |ws, cx| choice(ws, window, cx)).ok();
                }
            };
            dialog
                .w(px(420.))
                .title(title.clone())
                .child(message.clone())
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
                            Button::new("discard")
                                .small()
                                .label("Don't Save")
                                .on_click(choose(on_discard.clone())),
                        )
                        .child(
                            Button::new("save")
                                .primary()
                                .small()
                                .label("Save")
                                .on_click(choose(on_save.clone())),
                        ),
                )
        });
    }
}
