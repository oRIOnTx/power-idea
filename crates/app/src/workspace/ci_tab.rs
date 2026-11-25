//! The GitHub Actions tab: opening it, the HEAD status on its activity-bar icon, and what
//! the tab asks the workspace to do (open a file at an annotation, run a command in a terminal).

use std::path::Path;

use ci::State;
use gpui_kit::*;

use super::Workspace;
use crate::editor::CenterTab;
use crate::github_actions::{CiView, HeadChanged};

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
        // Only the badge depends on the tab: repaint the workspace when HEAD's status changes.
        let head = cx.subscribe(&view, |_, _, _: &HeadChanged, cx| cx.notify());
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
}
