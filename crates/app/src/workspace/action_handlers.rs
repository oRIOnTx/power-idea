//! Routes IDE actions (see `crate::actions`) to workspace methods.

use gpui_kit::*;

use super::{LeftPanel, Workspace};
use crate::actions::*;
use crate::editor;
use crate::file_ops::ClipMode;
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
        .on_action(cx.listener(|this, _: &Fetch, w, cx| this.fetch(w, cx)))
        .on_action(cx.listener(|this, _: &ResolveConflicts, w, cx| this.open_conflicts(None, w, cx)))
        .on_action(cx.listener(|this, _: &UpdateProject, w, cx| this.pull(w, cx)))
        .on_action(cx.listener(|this, _: &ShowGitLog, w, cx| this.toggle_git_log(w, cx)))
        .on_action(
            cx.listener(|this, action: &GitLogAction, w, cx| this.handle_git_log_action(action, w, cx)),
        )
        .on_action(cx.listener(|this, _: &Branches, w, cx| this.open_branches(w, cx)))
        .on_action(cx.listener(|this, _: &NewBranch, w, cx| {
            this.prompt_new_branch("HEAD".into(), String::new(), w, cx);
        }))
        .on_action(cx.listener(|this, _: &CheckoutRevision, w, cx| this.prompt_checkout_revision(w, cx)))
        .on_action(cx.listener(|_, _: &CopyMcpCommand, w, cx| crate::mcp_bridge::copy_claude_command(w, cx)))
        .on_action(cx.listener(|this, _: &OpenSettings, w, cx| this.open_settings(w, cx)))
        .on_action(cx.listener(|this, action: &FileAction, w, cx| this.handle_file_action(action, w, cx)))
        .on_action(cx.listener(|this, _: &CutFiles, w, cx| this.clip_selection(ClipMode::Cut, w, cx)))
        .on_action(cx.listener(|this, _: &CopyFiles, w, cx| this.clip_selection(ClipMode::Copy, w, cx)))
        .on_action(cx.listener(|this, _: &PasteFiles, w, cx| this.paste_into_selection(w, cx)))
        .on_action(cx.listener(|this, _: &DeleteFiles, w, cx| this.delete_selection(w, cx)))
        .on_action(cx.listener(|this, _: &NavigateBack, w, cx| this.navigate(false, w, cx)))
        .on_action(
            cx.listener(|this, action: &DatabaseAction, w, cx| this.handle_database_action(action, w, cx)),
        )
        .on_action(cx.listener(|this, _: &ToggleDockerView, w, cx| this.toggle_docker(w, cx)))
        .on_action(cx.listener(|this, _: &ToggleInsightsView, w, cx| this.toggle_insights(w, cx)))
        .on_action(cx.listener(|this, _: &ToggleDatabaseView, _, cx| {
            this.toggle_left_panel(LeftPanel::Database, cx);
        }))
        .on_action(cx.listener(|this, _: &NavigateForward, w, cx| this.navigate(true, w, cx)))
        .on_action(cx.listener(|this, _: &TogglePreview, w, cx| this.toggle_preview(w, cx)))
        .on_action(cx.listener(|this, _: &ReplaceInFiles, w, cx| this.open_replace_in_files(w, cx)))
        .on_action(cx.listener(|this, _: &RunConfiguration, w, cx| this.run_selected(w, cx)))
        .on_action(cx.listener(|this, _: &ChooseRunConfiguration, w, cx| this.choose_run_config(w, cx)))
        // Stop ends a debug session first, then run tabs.
        .on_action(cx.listener(|this, _: &StopRun, _, cx| {
            if this.debug_active() {
                this.stop_debug(cx);
            } else {
                this.stop_run(cx);
            }
        }))
        .on_action(cx.listener(|this, _: &DebugConfiguration, w, cx| this.debug_selected(w, cx)))
        .on_action(cx.listener(|this, _: &ToggleBreakpoint, _, cx| this.toggle_breakpoint(cx)))
        .on_action(cx.listener(|this, _: &DebugResume, _, cx| this.debug_step("continue", cx)))
        .on_action(cx.listener(|this, _: &DebugPause, _, cx| this.debug_step("pause", cx)))
        .on_action(cx.listener(|this, _: &StepOver, _, cx| this.debug_step("next", cx)))
        .on_action(cx.listener(|this, _: &StepInto, _, cx| this.debug_step("stepIn", cx)))
        .on_action(cx.listener(|this, _: &StepOut, _, cx| this.debug_step("stepOut", cx)))
        .on_action(cx.listener(|this, _: &EditRunConfigurations, w, cx| this.edit_run_configs(w, cx)))
        .on_action(cx.listener(|this, _: &FindUsages, w, cx| this.find_usages(w, cx)))
        .on_action(cx.listener(|this, _: &GotoClass, w, cx| this.goto_symbol(true, w, cx)))
        .on_action(cx.listener(|this, _: &GotoSymbol, w, cx| this.goto_symbol(false, w, cx)))
        .on_action(cx.listener(|this, _: &FileStructure, w, cx| this.file_structure(w, cx)))
        .on_action(cx.listener(|this, _: &RenameSymbol, w, cx| this.rename_symbol(w, cx)))
        .on_action(cx.listener(|this, _: &ReformatCode, w, cx| this.reformat_code(w, cx)))
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
