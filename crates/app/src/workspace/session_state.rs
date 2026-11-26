//! Saves the workspace state (`crate::session`) while the project is open and
//! restores it when the project is reopened.

use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;

use super::{LeftPanel, Workspace};
use crate::editor::CenterTab;
use crate::preview::PreviewMode;
use crate::session::{Session, SessionTab};

/// How often the state is compared with the saved copy (and written if changed).
const SAVE_INTERVAL: Duration = Duration::from_secs(2);

fn panel_name(panel: LeftPanel) -> &'static str {
    match panel {
        LeftPanel::Project => "project",
        LeftPanel::Commit => "commit",
        LeftPanel::Database => "database",
        LeftPanel::Docker => "docker",
        LeftPanel::Remote => "remote",
        LeftPanel::Insights => "insights",
    }
}

fn panel_from_name(name: &str) -> LeftPanel {
    match name {
        "commit" => LeftPanel::Commit,
        "database" => LeftPanel::Database,
        "docker" => LeftPanel::Docker,
        "remote" => LeftPanel::Remote,
        "insights" => LeftPanel::Insights,
        _ => LeftPanel::Project,
    }
}

fn mode_name(mode: PreviewMode) -> &'static str {
    match mode {
        PreviewMode::Editor => "editor",
        PreviewMode::Split => "split",
        PreviewMode::Preview => "preview",
    }
}

fn mode_from_name(name: &str) -> Option<PreviewMode> {
    match name {
        "editor" => Some(PreviewMode::Editor),
        "split" => Some(PreviewMode::Split),
        "preview" => Some(PreviewMode::Preview),
        _ => None,
    }
}

impl Workspace {
    fn session_snapshot(&self, cx: &App) -> Session {
        let mut tabs = Vec::new();
        let mut active = None;
        for (ix, tab) in self.tabs.iter().enumerate() {
            let entry = match tab {
                // A remote file needs a live connection: it is not reopened on start.
                CenterTab::Editor(tab) if tab.remote.is_some() => continue,
                CenterTab::Editor(tab) => {
                    let cursor = tab.state.read(cx).cursor_position();
                    SessionTab {
                        path: tab.rel.clone(),
                        line: cursor.line,
                        column: cursor.character,
                        preview: tab.preview.as_ref().map(|p| mode_name(p.mode).to_string()),
                    }
                }
                CenterTab::Image(tab) => SessionTab {
                    path: tab.rel.clone(),
                    line: 0,
                    column: 0,
                    preview: None,
                },
                CenterTab::Diff(_)
                | CenterTab::Settings
                | CenterTab::Conflicts
                | CenterTab::Console { .. }
                | CenterTab::Errors(_)
                | CenterTab::Ci(_) => continue,
            };
            if self.active_tab == Some(ix) {
                active = Some(tabs.len());
            }
            tabs.push(entry);
        }
        let mut expanded: Vec<String> = self.expanded.iter().cloned().collect();
        expanded.sort();
        let breakpoints = self
            .breakpoints
            .iter()
            .filter_map(|(path, lines)| {
                let rel = path.strip_prefix(&self.root).ok()?.to_str()?;
                Some((rel.to_string(), lines.iter().copied().collect()))
            })
            .collect();
        Session {
            tabs,
            active,
            expanded,
            recent_files: self.recent_files.clone(),
            show_sidebar: self.show_project_view,
            sidebar: panel_name(self.left_panel).into(),
            git_log_open: self.git_log_visible(),
            breakpoints,
        }
    }

    /// Reopens the tabs, folders and tool windows saved for this project.
    pub(super) fn restore_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut session = Session::load(&self.root);
        session.retain_existing(&self.root);
        self.load_pinned_terminals();

        self.expanded = session.expanded.iter().cloned().collect();
        self.refresh_tree(cx);
        self.show_project_view = session.show_sidebar;
        self.left_panel = panel_from_name(&session.sidebar);
        // The Docker panel connects lazily; reopened, it should connect right away.
        if self.show_project_view && self.left_panel == LeftPanel::Docker {
            self.ensure_docker(window, cx);
        }
        if self.show_project_view && self.left_panel == LeftPanel::Remote {
            self.ensure_remote(window, cx);
        }
        if self.show_project_view && self.left_panel == LeftPanel::Insights {
            if Self::insights_enabled(cx) {
                self.ensure_insights(window, cx);
            } else {
                self.left_panel = LeftPanel::Project;
            }
        }

        // Before the tabs open: opening an editor draws its breakpoint marks.
        self.breakpoints = session
            .breakpoints
            .iter()
            .map(|(rel, lines)| (self.root.join(rel), lines.iter().copied().collect()))
            .collect();

        // Reopening tabs is not navigation, so keep it out of Back/Forward history.
        self.nav_paused = true;
        for tab in &session.tabs {
            self.open_file(
                &tab.path,
                Some((tab.line as usize, tab.column as usize)),
                window,
                cx,
            );
            let mode = tab.preview.as_deref().and_then(mode_from_name);
            if let (Some(mode), Some(CenterTab::Editor(editor))) = (mode, self.tabs.last_mut())
                && let Some(preview) = editor.preview.as_mut()
            {
                preview.mode = mode;
            }
        }
        self.restore_unsaved(window, cx);
        let active = session.active.and_then(|ix| session.tabs.get(ix));
        if let Some(ix) = active.and_then(|a| self.tabs.iter().position(|t| t.file_rel() == Some(&a.path))) {
            self.activate_tab(ix, window, cx);
        }
        self.nav_paused = false;
        // Opening tabs reshuffles recent files; the saved order wins.
        if !session.recent_files.is_empty() {
            self.recent_files = session.recent_files.clone();
        }
        if session.git_log_open && self.git.is_some() {
            self.show_git_log(window, cx);
        }

        self.saved_session = Some(self.session_snapshot(cx));
        self.start_session_saver(cx);
    }

    /// Puts back text that was unsaved when the app last quit without asking.
    fn restore_unsaved(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(recovery) = crate::recovery::Recovery::new() else {
            return;
        };
        let mut restored = 0;
        for (rel, text) in recovery.take(&self.root) {
            if !self.root.join(&rel).is_file() {
                continue;
            }
            self.open_file(&rel, None, window, cx);
            let Some(tab) = self.active().filter(|tab| tab.rel == rel) else {
                continue;
            };
            if tab.saved_text.as_ref() != text.as_str() {
                let state = tab.state.clone();
                state.update(cx, |state, cx| state.set_value(text, window, cx));
                restored += 1;
            }
        }
        if restored > 0 {
            let text = format!(
                "Restored unsaved changes in {restored} file{} from the last session",
                if restored == 1 { "" } else { "s" }
            );
            window.push_notification(Notification::info(text), cx);
        }
    }

    /// The state to write, if it changed since the last save.
    fn session_to_save(&mut self, cx: &App) -> Option<(PathBuf, Session)> {
        // LightEdit must not leave `.power-idea/` behind in whatever folder a file came from.
        if self.light {
            return None;
        }
        let session = self.session_snapshot(cx);
        if self.saved_session.as_ref() == Some(&session) {
            return None;
        }
        self.saved_session = Some(session.clone());
        Some((self.root.clone(), session))
    }

    fn start_session_saver(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(SAVE_INTERVAL).await;
                let Ok(changed) = this.update(cx, |ws, cx| ws.session_to_save(cx)) else {
                    break;
                };
                if let Some((root, session)) = changed {
                    let saved = cx.background_spawn(async move { session.save(&root) }).await;
                    if let Err(err) = saved {
                        eprintln!("saving workspace state: {err:#}");
                    }
                }
            }
        })
        .detach();
    }

    /// Writes the state right away, e.g. when the window closes.
    pub(super) fn save_session_now(&mut self, cx: &App) {
        self.save_pinned_now(cx);
        if let Some((root, session)) = self.session_to_save(cx)
            && let Err(err) = session.save(&root)
        {
            eprintln!("saving workspace state: {err:#}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LeftPanel, mode_from_name, mode_name, panel_from_name, panel_name};
    use crate::preview::PreviewMode;

    #[test]
    fn names_round_trip() {
        for panel in [
            LeftPanel::Project,
            LeftPanel::Commit,
            LeftPanel::Database,
            LeftPanel::Docker,
            LeftPanel::Remote,
            LeftPanel::Insights,
        ] {
            assert!(panel_from_name(panel_name(panel)) == panel);
        }
        for mode in [PreviewMode::Editor, PreviewMode::Split, PreviewMode::Preview] {
            assert_eq!(mode_from_name(mode_name(mode)), Some(mode));
        }
        assert!(panel_from_name("bogus") == LeftPanel::Project);
        assert_eq!(mode_from_name("bogus"), None);
    }
}
