//! The pinned "Claude" terminal tab: `claude` in the project root with the injected settings
//! (Settings → Claude Code → Terminal). One per project window, always the first tab.

use std::collections::BTreeMap;

use claude_integration::launcher::{self, LaunchPlan};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::ButtonVariant;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;

use super::Workspace;
use crate::claude::{self, Binary};
use crate::config::AppConfig;
use crate::terminal::{Session, TerminalView};

impl Workspace {
    /// The Claude tab is switched on (and this is a project window).
    pub(super) fn wants_claude_tab(&self, cx: &App) -> bool {
        !self.light
            && cx
                .try_global::<claude::ClaudeState>()
                .is_some_and(|s| s.settings.enabled && s.settings.terminal.claude_tab)
    }

    pub(super) fn has_claude_tab(&self, cx: &App) -> bool {
        self.terminals.first().is_some_and(|t| t.read(cx).is_claude())
    }

    /// Opens the Claude tab first in the terminal panel (writing its generated files first).
    pub(super) fn start_claude_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = claude::state(cx);
        let terminal_settings = state.settings.terminal.clone();
        let binary = state.binary.clone();
        let facts = self.claude_facts();
        let Some(plan) = claude::plan_for(&facts, terminal_settings.continue_last, cx) else {
            return;
        };
        let command = match &binary {
            Binary::Found { .. } => {
                let start = command_line(&plan);
                if terminal_settings.auto_start {
                    start
                } else {
                    format!(
                        "printf '\\033[2mClaude Code: press Enter to start, or Ctrl-C to cancel.\\033[0m\\n'; \
                         read -r _ && exec {start}"
                    )
                }
            }
            Binary::Missing(reason) => missing_message(reason),
            Binary::Detecting => missing_message("claude is still being detected — use Restart in a moment"),
        };
        let env: BTreeMap<String, String> = plan.env.iter().cloned().collect();
        claude::remember_active_skills(&self.root, plan.skill_names.clone(), cx);
        cx.spawn_in(window, async move |this, cx| {
            let written = cx
                .background_spawn({
                    let plan = plan.clone();
                    async move { launcher::write(&plan) }
                })
                .await;
            this.update_in(cx, |ws, window, cx| {
                if let Err(err) = written {
                    let message = format!("Claude settings could not be written: {err:#}");
                    window.push_notification(Notification::error(message), cx);
                }
                ws.spawn_claude_terminal(&command, &env, window, cx);
            })
            .ok();
        })
        .detach();
    }

    fn spawn_claude_terminal(
        &mut self,
        command: &str,
        env: &BTreeMap<String, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.has_claude_tab(cx) {
            return;
        }
        let shell =
            Some(cx.global::<AppConfig>().0.terminal_shell.trim().to_string()).filter(|s| !s.is_empty());
        let root = self.root.clone();
        let session = match Session::spawn_command(root.clone(), shell, command, env) {
            Ok(session) => session,
            Err(err) => return self.notify_error(format!("Cannot start Claude: {err:#}"), window, cx),
        };
        let terminal = cx.new(|cx| {
            TerminalView::new(session, cx)
                .for_run("Claude".into())
                .claude_tab()
                .with_links(root.clone(), root)
        });
        self.insert_terminal(0, terminal, window, cx);
    }

    /// Stops the Claude session (if any) and starts a new one.
    pub(super) fn restart_claude_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.has_claude_tab(cx) {
            if let Some(tab) = self.terminals.first() {
                tab.update(cx, TerminalView::stop);
            }
            self.close_terminal(0, window, cx);
        }
        self.start_claude_tab(window, cx);
    }

    /// Closes a terminal tab; a running Claude session asks first.
    pub(super) fn close_terminal_asking(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let running_claude = self
            .terminals
            .get(ix)
            .is_some_and(|t| t.read(cx).is_claude() && t.read(cx).is_running());
        if !running_claude {
            return self.close_terminal(ix, window, cx);
        }
        let workspace = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let workspace = workspace.clone();
            dialog
                .confirm()
                .title("Stop the Claude session?")
                .description(
                    "Claude Code is still running in this tab. You can continue it later with --continue.",
                )
                .footer(crate::style::confirm_buttons(
                    "Stop and Close",
                    ButtonVariant::Danger,
                ))
                .on_ok(move |_, window, cx| {
                    workspace
                        .update(cx, |ws, cx| {
                            if let Some(tab) = ws.terminals.first().filter(|t| t.read(cx).is_claude()) {
                                tab.update(cx, TerminalView::stop);
                                ws.close_terminal(0, window, cx);
                            }
                        })
                        .ok();
                    true
                })
        });
    }
}

/// The shell command that starts `claude` with the plan's arguments.
fn command_line(plan: &LaunchPlan) -> String {
    std::iter::once(plan.program.to_string_lossy().into_owned())
        .chain(plan.args.iter().cloned())
        .map(|a| launcher::shell_quote(&a))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Printed in the tab when there is no `claude` to start.
fn missing_message(reason: &str) -> String {
    let text = format!(
        "{reason}.\\n\\nInstall Claude Code:  npm install -g @anthropic-ai/claude-code\\n\
         Docs: https://docs.claude.com/en/docs/claude-code\\n\
         Then set its path in Settings → Claude Code if it is not on PATH, and press Restart in this tab.\\n"
    );
    format!("printf {}", launcher::shell_quote(&text))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use claude_integration::launcher::LaunchPlan;

    use super::{command_line, missing_message};

    #[test]
    fn quotes_the_command_line() {
        let plan = LaunchPlan {
            program: PathBuf::from("/opt/claude bin/claude"),
            args: vec!["--append-system-prompt-file".into(), "/c/rules.md".into()],
            env: vec![],
            cwd: PathBuf::from("/p"),
            files: vec![],
            plugin: None,
            skill_names: vec![],
        };
        assert_eq!(
            command_line(&plan),
            "'/opt/claude bin/claude' --append-system-prompt-file /c/rules.md"
        );
        assert!(missing_message("claude was not found").starts_with("printf 'claude was not found."));
    }
}
