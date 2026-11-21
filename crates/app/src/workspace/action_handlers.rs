//! Routes IDE actions (see `crate::actions`) to workspace methods.

use gpui_kit::*;

use super::{LeftPanel, Workspace};
use crate::actions::*;
use crate::editor;
use crate::palette::PaletteMode;

impl Workspace {
    pub(super) fn register_actions(&self, el: Div, cx: &mut Context<Self>) -> Div {
        el.on_action(cx.listener(|this, _: &SearchEverywhere, w, cx| {
            this.open_palette(PaletteMode::SearchEverywhere, w, cx);
        }))
        .on_action(cx.listener(|this, _: &GotoFile, w, cx| this.open_palette(PaletteMode::GotoFile, w, cx)))
        .on_action(
            cx.listener(|this, _: &RecentFiles, w, cx| this.open_palette(PaletteMode::RecentFiles, w, cx)),
        )
        .on_action(
            cx.listener(|this, _: &FindInFiles, w, cx| this.open_palette(PaletteMode::FindInFiles, w, cx)),
        )
        .on_action(
            cx.listener(|this, _: &FindAction, w, cx| this.open_palette(PaletteMode::FindAction, w, cx)),
        )
        .on_action(cx.listener(|this, _: &RecentProjects, w, cx| {
            this.open_palette(PaletteMode::RecentProjects, w, cx);
        }))
        .on_action(cx.listener(|this, _: &GotoLine, w, cx| {
            if this.active().is_some() {
                this.open_palette(PaletteMode::GotoLine, w, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &SaveFile, w, cx| this.save_active(w, cx)))
        .on_action(cx.listener(|this, _: &CloseTab, w, cx| {
            if let Some(ix) = this.active_tab {
                this.close_tab(ix, w, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &NextTab, w, cx| {
            if let Some(ix) = this.active_tab {
                this.activate_tab((ix + 1) % this.tabs.len(), w, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &PrevTab, w, cx| {
            if let Some(ix) = this.active_tab {
                this.activate_tab(ix.checked_sub(1).unwrap_or(this.tabs.len() - 1), w, cx);
            }
        }))
        .on_action(
            cx.listener(|this, _: &ToggleProjectView, _, cx| this.toggle_left_panel(LeftPanel::Project, cx)),
        )
        .on_action(
            cx.listener(|this, _: &ToggleCommitView, _, cx| this.toggle_left_panel(LeftPanel::Commit, cx)),
        )
        .on_action(cx.listener(|this, _: &CommitChanges, w, cx| {
            this.show_project_view = true;
            this.left_panel = LeftPanel::Commit;
            this.focus_commit_message(w, cx);
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &Push, w, cx| this.push(w, cx)))
        .on_action(cx.listener(|this, _: &UpdateProject, w, cx| this.pull(w, cx)))
        .on_action(cx.listener(|this, _: &ShowGitLog, w, cx| this.open_git_log(w, cx)))
        .on_action(cx.listener(|this, _: &Branches, w, cx| this.open_branches(w, cx)))
        .on_action(cx.listener(|_, _: &CopyMcpCommand, w, cx| crate::mcp_bridge::copy_claude_command(w, cx)))
        .on_action(cx.listener(|this, _: &OpenSettings, w, cx| this.open_settings(w, cx)))
        .on_action(cx.listener(|this, action: &FileAction, w, cx| this.handle_file_action(action, w, cx)))
        .on_action(cx.listener(|this, _: &NavigateBack, w, cx| this.navigate(false, w, cx)))
        .on_action(
            cx.listener(|this, action: &DatabaseAction, w, cx| this.handle_database_action(action, w, cx)),
        )
        .on_action(cx.listener(|this, _: &ToggleDatabaseView, _, cx| {
            this.toggle_left_panel(LeftPanel::Database, cx);
        }))
        .on_action(cx.listener(|this, _: &NavigateForward, w, cx| this.navigate(true, w, cx)))
        .on_action(cx.listener(|this, _: &CloseWindow, w, cx| {
            if this.can_close_window(w, cx) {
                w.remove_window();
            }
        }))
        .on_action(cx.listener(|this, _: &ToggleTerminal, w, cx| this.toggle_terminal(w, cx)))
        .on_action(cx.listener(|this, _: &NewTerminal, w, cx| this.new_terminal(w, cx)))
        .on_action(
            cx.listener(|this, _: &CloseTerminal, w, cx| this.close_terminal(this.active_terminal, w, cx)),
        )
        .on_action(cx.listener(|this, _: &ToggleLineComment, w, cx| {
            this.edit_lines(w, cx, |text, line, lang| {
                editor::toggle_line_comment(text, line, editor::line_comment_prefix(lang)?)
            });
        }))
        .on_action(cx.listener(|this, _: &DuplicateLine, w, cx| {
            this.edit_lines(w, cx, |text, line, _| editor::duplicate_line(text, line));
        }))
        .on_action(cx.listener(|this, _: &DeleteLine, w, cx| {
            this.edit_lines(w, cx, |text, line, _| editor::delete_line(text, line));
        }))
        .on_action(cx.listener(|this, _: &MoveLineUp, w, cx| {
            this.edit_lines(w, cx, |text, line, _| editor::move_line(text, line, true));
        }))
        .on_action(cx.listener(|this, _: &MoveLineDown, w, cx| {
            this.edit_lines(w, cx, |text, line, _| editor::move_line(text, line, false));
        }))
    }
}
