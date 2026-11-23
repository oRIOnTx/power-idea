//! Opening the Errors tab and jumping to where a recorded error happened.

use std::process::Command;

use gpui_kit::*;

use super::Workspace;
use crate::editor::CenterTab;
use crate::errors::{self, ErrorsView};

impl Workspace {
    /// Shows the Errors tab (one per window); activating it marks every error as seen.
    pub(crate) fn open_errors(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let existing = self.tabs.iter().position(|t| matches!(t, CenterTab::Errors(_)));
        let ix = existing.unwrap_or_else(|| {
            let workspace = cx.entity().downgrade();
            self.tabs
                .push(CenterTab::Errors(cx.new(|cx| ErrorsView::new(workspace, cx))));
            self.tabs.len() - 1
        });
        self.activate_tab(ix, window, cx);
    }

    /// Opens a project file at the recorded line; reveals anything else in Finder.
    pub(crate) fn open_error_location(
        &mut self,
        location: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let home = std::env::home_dir();
        let Some((path, line, column)) = errors::resolve_location(location, &self.root, home.as_deref())
        else {
            return self.notify_error(format!("Not a file location: {location}"), window, cx);
        };
        if !path.is_file() {
            return self.notify_error(
                format!("{} does not exist on this machine", path.display()),
                window,
                cx,
            );
        }
        if path.starts_with(&self.root) {
            self.jump_to(&path, line, column, window, cx);
        } else if let Err(err) = Command::new("open").arg("-R").arg(&path).spawn() {
            self.notify_error(format!("Cannot reveal {}: {err}", path.display()), window, cx);
        }
    }
}
