//! Renaming and pinning terminal tabs (tab context menu, double-click), and the pinned
//! tabs saved per project (`crate::pinned_terminals`), restored when the panel first opens.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;

use super::Workspace;
use crate::actions::{TerminalTabAction, TerminalTabOp};
use crate::pinned_terminals::{self, PinnedStore, PinnedTab};
use crate::terminal::{self, Session, TerminalView};

/// Pin and rename bursts (several tabs, typing a name) end up in one write.
const SAVE_DELAY: Duration = Duration::from_millis(500);
/// How often the working directories of pinned tabs are checked.
const CWD_POLL: Duration = Duration::from_secs(3);

/// Pinned-tab bookkeeping of a workspace.
#[derive(Default)]
pub(super) struct PinState {
    /// Saved tabs not reopened yet; kept in every save until they are.
    saved: Vec<PinnedTab>,
    /// The saved tabs were reopened (or the panel opened with none): never twice.
    restored: bool,
    /// The newest list waiting for the debounced write.
    pending: Option<Vec<PinnedTab>>,
    saver: Option<Task<()>>,
    poller: Option<Task<()>>,
    /// The tab being renamed and its inline editor.
    pub(super) renaming: Option<(EntityId, Entity<InputState>)>,
}

impl PinState {
    /// Whether the panel should reopen saved tabs now (true once per project session).
    fn take_restore(&mut self) -> Vec<PinnedTab> {
        if self.restored {
            return Vec::new();
        }
        self.restored = true;
        std::mem::take(&mut self.saved)
    }
}

/// An open tab as the save sees it: pinned, renamed title, encoded directory.
type OpenTab = (bool, Option<String>, String);

/// The tabs to save: open pinned tabs in order, then saved ones not reopened yet.
/// Unpinned tabs, renamed or not, are not saved.
fn snapshot(open: &[OpenTab], not_reopened: &[PinnedTab]) -> Vec<PinnedTab> {
    open.iter()
        .filter(|(pinned, ..)| *pinned)
        .map(|(_, name, cwd)| (name.clone(), cwd.clone()))
        .chain(not_reopened.iter().map(|t| (t.name.clone(), t.cwd.clone())))
        .enumerate()
        .map(|(order, (name, cwd))| PinnedTab { name, cwd, order })
        .collect()
}

impl Workspace {
    /// Reads the saved pinned tabs of this project (once, when it opens).
    pub(super) fn load_pinned_terminals(&mut self) {
        if self.light {
            return;
        }
        let Some(store) = PinnedStore::for_project(&self.root) else {
            return;
        };
        match store.load() {
            Ok(tabs) => self.pins.saved = tabs,
            Err(err) => error_journal::report(&err, "loading pinned terminal tabs"),
        }
    }

    /// Reopens the saved pinned tabs (fresh shells in their directories) the first time
    /// the panel opens; false when there is nothing to reopen.
    pub(super) fn restore_pinned_terminals(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let saved = self.pins.take_restore();
        if saved.is_empty() {
            return false;
        }
        let shell = Self::configured_shell(cx);
        let mut at = self.terminals.iter().filter(|t| t.read(cx).is_pinned()).count();
        let first = at;
        for tab in saved {
            let cwd = pinned_terminals::resolve_cwd(&self.root, &tab.cwd);
            let session = match Session::spawn(cwd.clone(), shell.clone()) {
                Ok(session) => session,
                Err(err) => {
                    self.notify_error(format!("Cannot start terminal: {err:#}"), window, cx);
                    continue;
                }
            };
            let root = self.root.clone();
            let view = cx.new(|cx| {
                let mut view = TerminalView::new(session, cx)
                    .with_links(root, cwd.clone())
                    .in_dir(cwd);
                view.set_pinned(true);
                if let Some(name) = &tab.name {
                    view.set_name(name);
                }
                view
            });
            self.insert_terminal(at, view, window, cx);
            at += 1;
        }
        self.active_terminal = first.min(self.terminals.len().saturating_sub(1));
        self.focus_terminal(window, cx);
        self.watch_pinned_cwds(cx);
        cx.notify();
        at > first
    }

    pub(super) fn terminal_tab_menu(
        &self,
        menu: PopupMenu,
        tab: &Entity<TerminalView>,
        cx: &App,
    ) -> PopupMenu {
        let id = tab.entity_id().as_u64();
        let action = |op| Box::new(TerminalTabAction { op, tab: id }) as Box<dyn Action>;
        let view = tab.read(cx);
        let menu = menu
            .action_context(self.focus_handle.clone())
            .menu("Rename…", action(TerminalTabOp::Rename));
        let menu = match (view.can_pin(), view.is_pinned()) {
            (true, false) => menu.menu("Pin Tab", action(TerminalTabOp::Pin)),
            (_, true) => menu.menu("Unpin Tab", action(TerminalTabOp::Unpin)),
            (false, false) => menu,
        };
        menu.separator()
            .menu("Close Tab", action(TerminalTabOp::Close))
            .menu("Close Other Tabs", action(TerminalTabOp::CloseOthers))
    }

    pub(super) fn handle_terminal_tab_action(
        &mut self,
        action: &TerminalTabAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self
            .terminals
            .iter()
            .position(|t| t.entity_id().as_u64() == action.tab)
        else {
            return;
        };
        match action.op {
            TerminalTabOp::Rename => self.start_terminal_rename(ix, window, cx),
            TerminalTabOp::Pin => self.set_terminal_pinned(ix, true, cx),
            TerminalTabOp::Unpin => self.set_terminal_pinned(ix, false, cx),
            TerminalTabOp::Close => self.close_terminal_from_menu(ix, window, cx),
            TerminalTabOp::CloseOthers => self.close_other_terminals(ix, window, cx),
        }
    }

    fn set_terminal_pinned(&mut self, ix: usize, pinned: bool, cx: &mut Context<Self>) {
        let Some(tab) = self.terminals.get(ix).cloned() else {
            return;
        };
        tab.update(cx, |t, _| t.set_pinned(pinned));
        let active = self.terminals.get(self.active_terminal).cloned();
        pinned_terminals::pinned_first(&mut self.terminals, |t| t.read(cx).is_pinned());
        self.active_terminal = active
            .and_then(|a| self.terminals.iter().position(|t| *t == a))
            .unwrap_or(0);
        if pinned {
            self.watch_pinned_cwds(cx);
        }
        self.schedule_pinned_save(cx);
        cx.notify();
    }

    /// "Close Tab" from the menu also closes a pinned tab (it is unpinned first).
    fn close_terminal_from_menu(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let pinned_tab = self.terminals.get(ix).filter(|t| t.read(cx).is_pinned()).cloned();
        let was_pinned = pinned_tab.is_some();
        if let Some(tab) = pinned_tab {
            tab.update(cx, |t, _| t.set_pinned(false));
        }
        self.close_terminal_asking(ix, window, cx);
        if was_pinned {
            self.schedule_pinned_save(cx);
        }
    }

    /// Closes every other tab except pinned ones.
    fn close_other_terminals(&mut self, keep: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(kept) = self.terminals.get(keep).cloned() else {
            return;
        };
        let others: Vec<_> = self
            .terminals
            .iter()
            .filter(|t| **t != kept && !t.read(cx).is_pinned())
            .cloned()
            .collect();
        for tab in others {
            if let Some(ix) = self.terminals.iter().position(|t| *t == tab) {
                self.close_terminal_asking(ix, window, cx);
            }
        }
        if let Some(ix) = self.terminals.iter().position(|t| *t == kept) {
            self.active_terminal = ix;
        }
        cx.notify();
    }

    pub(super) fn start_terminal_rename(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.terminals.get(ix).cloned() else {
            return;
        };
        let initial = tab.read(cx).display_name().to_string();
        let input = cx.new(|cx| InputState::new(window, cx).default_value(initial));
        let id = tab.entity_id();
        let workspace = cx.entity().downgrade();
        // `window.subscribe`, not `cx.subscribe_in`: finishing updates the workspace, which
        // must not happen inside one of its own updates.
        window
            .subscribe(&input, cx, move |_, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    workspace
                        .update(cx, |ws, cx| ws.finish_terminal_rename(id, true, window, cx))
                        .ok();
                }
            })
            .detach();
        input.update(cx, |i, cx| i.focus(window, cx));
        self.pins.renaming = Some((id, input));
        cx.notify();
    }

    /// Applies (`save`) or drops the inline rename of tab `id`.
    pub(super) fn finish_terminal_rename(
        &mut self,
        id: EntityId,
        save: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((_, input)) = self.pins.renaming.take_if(|(r, _)| *r == id) else {
            return;
        };
        let tab = self.terminals.iter().find(|t| t.entity_id() == id).cloned();
        if let (true, Some(tab)) = (save, tab) {
            let name = input.read(cx).value().to_string();
            let pinned = tab.update(cx, |t, _| {
                t.set_name(&name);
                t.is_pinned()
            });
            if pinned {
                self.schedule_pinned_save(cx);
            }
        }
        self.focus_terminal(window, cx);
        cx.notify();
    }

    fn pinned_snapshot(&self, cx: &App) -> Vec<PinnedTab> {
        let open: Vec<OpenTab> = self
            .terminals
            .iter()
            .map(|t| t.read(cx))
            .map(|t| {
                let cwd = t.cwd().map(|dir| pinned_terminals::encode_cwd(&self.root, dir));
                (
                    t.is_pinned(),
                    t.custom_name().map(str::to_string),
                    cwd.unwrap_or_default(),
                )
            })
            .collect();
        snapshot(&open, &self.pins.saved)
    }

    /// Writes the pinned tabs shortly, off the UI thread, as they are now.
    fn schedule_pinned_save(&mut self, cx: &mut Context<Self>) {
        if self.light {
            return;
        }
        self.pins.pending = Some(self.pinned_snapshot(cx));
        if self.pins.saver.is_some() {
            return;
        }
        let Some(store) = PinnedStore::for_project(&self.root) else {
            return;
        };
        self.pins.saver = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_DELAY).await;
            let Ok(Some(tabs)) = this.update(cx, |ws, _| {
                ws.pins.saver = None;
                ws.pins.pending.take()
            }) else {
                return;
            };
            if let Err(err) = cx.background_spawn(async move { store.save(&tabs) }).await {
                error_journal::report(&err, "saving pinned terminal tabs");
            }
        }));
    }

    /// The final write when the project closes (with the last known directories).
    /// Skipped while nothing was reopened or changed, so the saved list stays intact.
    pub(super) fn save_pinned_now(&mut self, cx: &App) {
        if self.light || (!self.pins.restored && self.pins.pending.is_none()) {
            return;
        }
        self.pins.pending = None;
        self.pins.saver = None;
        let tabs = self.pinned_snapshot(cx);
        if let Some(store) = PinnedStore::for_project(&self.root)
            && let Err(err) = store.save(&tabs)
        {
            error_journal::report(&err, "saving pinned terminal tabs");
        }
    }

    /// Keeps the working directories of pinned tabs current while any are open.
    fn watch_pinned_cwds(&mut self, cx: &mut Context<Self>) {
        if self.pins.poller.is_some() {
            return;
        }
        self.pins.poller = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(CWD_POLL).await;
                let Ok(pids) = this.update(cx, |ws, cx| ws.pinned_shell_pids(cx)) else {
                    return;
                };
                if pids.is_empty() {
                    this.update(cx, |ws, _| ws.pins.poller = None).ok();
                    return;
                }
                let dirs = cx
                    .background_spawn(async move { terminal::shell_cwds(&pids) })
                    .await;
                if this.update(cx, |ws, cx| ws.apply_pinned_cwds(&dirs, cx)).is_err() {
                    return;
                }
            }
        }));
    }

    fn pinned_shell_pids(&self, cx: &App) -> Vec<u32> {
        self.terminals
            .iter()
            .map(|t| t.read(cx))
            .filter(|t| t.is_pinned())
            .map(TerminalView::shell_pid)
            .collect()
    }

    fn apply_pinned_cwds(&mut self, dirs: &HashMap<u32, PathBuf>, cx: &mut Context<Self>) {
        let mut changed = false;
        for tab in self.terminals.clone() {
            let view = tab.read(cx);
            let Some(dir) = dirs.get(&view.shell_pid()).filter(|_| view.is_pinned()) else {
                continue;
            };
            if view.cwd() != Some(dir.as_path()) {
                let dir = dir.clone();
                tab.update(cx, |t, _| t.set_cwd(dir));
                changed = true;
            }
        }
        if changed {
            self.schedule_pinned_save(cx);
        }
    }

    /// ⌘W and the tab's × do not close pinned tabs.
    pub(super) fn guard_pinned_close(&self, ix: usize, window: &mut Window, cx: &mut App) -> bool {
        let pinned = self.terminals.get(ix).is_some_and(|t| t.read(cx).is_pinned());
        if pinned {
            let text = "This tab is pinned: unpin it or use Close Tab from its menu.";
            window.push_notification(Notification::info(text), cx);
        }
        pinned
    }
}

#[cfg(test)]
mod tests {
    use super::{PinState, snapshot};
    use crate::pinned_terminals::PinnedTab;

    fn saved(name: &str) -> PinnedTab {
        PinnedTab {
            name: Some(name.into()),
            cwd: "web".into(),
            order: 0,
        }
    }

    #[test]
    fn restores_once_per_session() {
        let mut state = PinState {
            saved: vec![saved("frontend")],
            ..PinState::default()
        };
        assert_eq!(state.take_restore().len(), 1);
        assert!(
            state.take_restore().is_empty(),
            "a second click must not duplicate tabs"
        );
    }

    #[test]
    fn snapshot_orders_open_tabs_then_not_reopened_ones() {
        let open = vec![
            (true, Some("api".to_string()), String::new()),
            (true, None, "web".to_string()),
        ];
        let tabs = snapshot(&open, &[saved("later")]);
        let orders: Vec<_> = tabs.iter().map(|t| (t.order, t.name.clone())).collect();
        assert_eq!(
            orders,
            vec![(0, Some("api".into())), (1, None), (2, Some("later".into()))]
        );
    }

    #[test]
    fn renames_persist_only_for_pinned_tabs() {
        let open = vec![
            (false, Some("renamed".to_string()), String::new()),
            (true, Some("kept".to_string()), "web".to_string()),
            (false, None, String::new()),
        ];
        let tabs = snapshot(&open, &[]);
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs[0].name.as_deref(), Some("kept"));
        assert_eq!((tabs[0].cwd.as_str(), tabs[0].order), ("web", 0));
    }
}
