//! "Claude" terminal tabs: `claude` in the project root, with the IDE's MCP server and live
//! gauges when Settings → Claude Code is on.

use std::collections::BTreeMap;

use claude_integration::launcher::{self, LaunchPlan, Start};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::ButtonVariant;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;

use super::Workspace;
use crate::claude::{self, Binary};
use crate::terminal::{Session, TerminalView};

impl Workspace {
    /// Opens a Claude tab at `ix` (the end when `None`), writing its generated files first.
    pub(super) fn start_claude_tab(
        &mut self,
        ix: Option<usize>,
        start: Start,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let binary = claude::state(cx).binary.clone();
        let Some(plan) = claude::plan_for(&self.root, start, cx) else {
            return;
        };
        let command = match &binary {
            Binary::Found { .. } => command_line(&plan),
            Binary::Missing(reason) => missing_message(reason),
            Binary::Detecting => missing_message("claude is still being detected — use Restart in a moment"),
        };
        let env: BTreeMap<String, String> = plan.env.iter().cloned().collect();
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
                ws.spawn_claude_terminal(ix, &command, &env, window, cx);
            })
            .ok();
        })
        .detach();
    }

    fn spawn_claude_terminal(
        &mut self,
        ix: Option<usize>,
        command: &str,
        env: &BTreeMap<String, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let shell = Self::configured_shell(cx);
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
        let ix = ix.unwrap_or(self.terminals.len());
        self.insert_terminal(ix, terminal, window, cx);
    }

    /// Stops the Claude session in tab `ix` and starts a new one in its place.
    pub(super) fn restart_claude_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.terminals.get(ix).filter(|t| t.read(cx).is_claude()).cloned() else {
            return;
        };
        tab.update(cx, TerminalView::stop);
        self.close_terminal(ix, window, cx);
        self.start_claude_tab(Some(ix), Start::New, window, cx);
    }

    /// Closes a terminal tab; a running Claude session asks first, a pinned tab stays.
    pub(super) fn close_terminal_asking(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.guard_pinned_close(ix, window, cx) {
            return;
        }
        let Some(tab) = self
            .terminals
            .get(ix)
            .filter(|t| t.read(cx).is_claude() && t.read(cx).is_running())
            .cloned()
        else {
            return self.close_terminal(ix, window, cx);
        };
        let workspace = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let workspace = workspace.clone();
            let tab = tab.clone();
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
                            // Tabs may have moved while the dialog was open.
                            if let Some(ix) = ws.terminals.iter().position(|t| *t == tab) {
                                tab.update(cx, TerminalView::stop);
                                ws.close_terminal(ix, window, cx);
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
            args: vec!["--mcp-config".into(), "/c/my mcp.json".into()],
            env: vec![],
            cwd: PathBuf::from("/p"),
            files: vec![],
        };
        assert_eq!(
            command_line(&plan),
            "'/opt/claude bin/claude' --mcp-config '/c/my mcp.json'"
        );
        assert!(missing_message("claude was not found").starts_with("printf 'claude was not found."));
    }
}
