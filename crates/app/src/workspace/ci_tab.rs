//! The GitHub Actions tab: opening it, the HEAD status on its activity-bar icon, and what
//! the tab asks the workspace to do (open a file at an annotation, run a command in a terminal).

use std::path::Path;
use std::time::Duration;

use ci::State;
use claude_integration::launcher::Start;
use gpui_kit::*;

use super::Workspace;
use crate::editor::CenterTab;
use crate::github_actions::{CiEvent, CiView};
use gpui_kit::component::notification::Notification;

pub(super) struct CiHandle {
    pub(super) view: Entity<CiView>,
    _head: Subscription,
}

impl Workspace {
    /// Shows the GitHub Actions tab (one per window).
    pub(crate) fn open_ci(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = self.ensure_ci(window, cx);
        let existing = self.tabs.iter().position(|t| matches!(t, CenterTab::Ci(_)));
        let ix = existing.unwrap_or_else(|| {
            self.tabs.push(CenterTab::Ci(view));
            self.tabs.len() - 1
        });
        self.activate_tab(ix, window, cx);
    }

    fn ensure_ci(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<CiView> {
        if let Some(handle) = &self.ci {
            return handle.view.clone();
        }
        let (workspace, root) = (cx.entity().downgrade(), self.root.clone());
        let view = cx.new(|cx| CiView::new(workspace, root, window, cx));
        // Only the badge and notifications reach the workspace; the rest stays in the tab.
        let head = cx.subscribe(&view, |ws, _, event: &CiEvent, cx| {
            if let CiEvent::Notify { title, body, failed } = event {
                let notification = if *failed {
                    Notification::error(body.clone())
                } else {
                    Notification::success(body.clone())
                };
                ws.pending_notices
                    .push(super::Notice::Plain(Box::new(notification.title(title.clone()))));
            }
            cx.notify();
        });
        self.ci = Some(CiHandle {
            view: view.clone(),
            _head: head,
        });
        view
    }

    /// HEAD's CI status for the activity bar, once the tab has been opened.
    pub(super) fn ci_head_state(&self, cx: &App) -> Option<State> {
        self.ci.as_ref().and_then(|h| h.view.read(cx).head_state())
    }

    pub(super) fn ci_tab_active(&self) -> bool {
        matches!(
            self.active_tab.and_then(|ix| self.tabs.get(ix)),
            Some(CenterTab::Ci(_))
        )
    }

    /// Tells the tab whether it is on screen (it polls faster then). Deferred: a tab switch can
    /// start inside the tab's own update (opening a file from an annotation).
    pub(super) fn sync_ci_visibility(&self, cx: &mut Context<Self>) {
        let Some(handle) = &self.ci else { return };
        let (view, visible) = (handle.view.clone(), self.ci_tab_active());
        cx.defer(move |cx| view.update(cx, |view, cx| view.set_visible(visible, cx)));
    }

    pub(crate) fn ci_open_location(
        &mut self,
        path: &Path,
        line: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Annotation paths come from GitHub: keep them inside the project.
        let inside = path
            .canonicalize()
            .ok()
            .filter(|p| p.is_file() && self.root.canonicalize().is_ok_and(|root| p.starts_with(root)));
        let Some(path) = inside else {
            return self.notify_error(format!("{} is not in this project", path.display()), window, cx);
        };
        self.jump_to(&path, line, 0, window, cx);
    }

    pub(crate) fn ci_run_in_terminal(&mut self, command: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.open_terminal_with_input(command, window, cx);
    }

    /// Puts `prompt` into the project's Claude tab (starting one if needed) without sending it,
    /// once Claude's input is ready; falls back to the clipboard.
    pub(crate) fn ci_send_to_claude(&mut self, prompt: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.light {
            return;
        }
        match self.current_claude_tab(cx) {
            Some(ix) => {
                self.active_terminal = ix;
                self.show_terminal_panel();
                self.focus_terminal(window, cx);
            }
            None => self.start_claude_tab(None, Start::New, window, cx),
        }
        cx.spawn_in(window, async move |this, cx| {
            for _ in 0..CLAUDE_READY_POLLS {
                let pasted = this
                    .update_in(cx, |ws, window, cx| ws.paste_into_claude(&prompt, window, cx))
                    .unwrap_or(true);
                if pasted {
                    return;
                }
                cx.background_executor().timer(CLAUDE_READY_EVERY).await;
            }
            this.update_in(cx, |ws, window, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(prompt.clone()));
                ws.notify_error(
                    "Claude is not ready yet; the CI context is on the clipboard, paste it with ⌘V".into(),
                    window,
                    cx,
                );
            })
            .ok();
        })
        .detach();
    }

    /// Pastes into the Claude tab if its prompt takes pasted text yet.
    fn paste_into_claude(&mut self, prompt: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(ix) = self.current_claude_tab(cx) else {
            return false;
        };
        let Some(terminal) = self.terminals.get(ix).cloned() else {
            return false;
        };
        if !terminal.read(cx).accepts_paste() {
            return false;
        }
        terminal.read(cx).paste_prompt(prompt);
        self.active_terminal = ix;
        self.show_terminal_panel();
        self.focus_terminal(window, cx);
        cx.notify();
        true
    }
}

/// A new Claude session takes a few seconds to show its prompt.
const CLAUDE_READY_POLLS: u32 = 80;
const CLAUDE_READY_EVERY: Duration = Duration::from_millis(250);
