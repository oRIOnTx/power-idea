//! Warns before closing a project window or quitting while terminals still run
//! programs (a server, a build, a run configuration): closing ends them.

use std::fmt::Write as _;

use gpui_kit::component::{ActiveTheme as _, WindowExt as _};
use gpui_kit::*;

use super::{Workspace, Workspaces};
use crate::style;
use crate::terminal;

/// What a workspace's terminals run, gathered on the UI thread.
#[derive(Default)]
struct Probe {
    /// Run configurations still running (known without asking the OS).
    runs: Vec<String>,
    /// Shell PIDs of plain terminals; `ps` tells whether they run something.
    shells: Vec<u32>,
}

impl Probe {
    fn is_empty(&self) -> bool {
        self.runs.is_empty() && self.shells.is_empty()
    }

    /// Program names, deduplicated in order. Blocks on `ps`.
    fn programs(self) -> Vec<String> {
        let mut names = self.runs;
        names.extend(terminal::foreground_programs(&self.shells));
        let mut seen = std::collections::HashSet::new();
        names.retain(|name| seen.insert(name.clone()));
        names
    }

    fn extend(&mut self, other: Probe) {
        self.runs.extend(other.runs);
        self.shells.extend(other.shells);
    }
}

/// Dialog text for the running programs.
fn describe(programs: &[String]) -> String {
    const SHOWN: usize = 3;
    let mut list = programs
        .iter()
        .take(SHOWN)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if programs.len() > SHOWN {
        let _ = write!(list, " and {} more", programs.len() - SHOWN);
    }
    format!("Running in the terminal: {list}. Closing will stop it.")
}

/// Asks "Terminate?"; calls `on_ok` when the user agrees.
fn ask(
    title: &'static str,
    programs: &[String],
    on_ok: impl Fn(&mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let message = describe(programs);
    let on_ok = std::rc::Rc::new(on_ok);
    window.open_alert_dialog(cx, move |dialog, _, cx| {
        let on_ok = on_ok.clone();
        dialog
            .confirm()
            .width(px(420.))
            .icon(style::lucide("square-terminal").text_color(cx.theme().primary))
            .title(title)
            .description(message.clone())
            .footer(style::confirm_buttons(
                "Terminate",
                component::button::ButtonVariant::Primary,
            ))
            .on_ok(move |_, window, cx| {
                on_ok(window, cx);
                true
            })
    });
}

/// ⌘Q: checks every project's terminals, asks if anything runs, then continues
/// with `quit` (`true` when the user already confirmed exiting).
pub(super) fn check_before_quit(quit: fn(bool, &mut App), cx: &mut App) {
    let mut probe = Probe::default();
    for (_, ws) in Workspaces::live(cx) {
        probe.extend(ws.read(cx).terminal_probe(cx));
    }
    if probe.is_empty() {
        return quit(false, cx);
    }
    let programs = cx.background_spawn(async move { probe.programs() });
    cx.spawn(async move |cx| {
        let programs = programs.await;
        cx.update(|cx| {
            if programs.is_empty() {
                return quit(false, cx);
            }
            let Some(handle) = cx.active_window().or_else(|| cx.windows().first().copied()) else {
                return quit(true, cx);
            };
            let shown = handle.update(cx, |_, window, cx| {
                window.activate_window();
                ask(
                    "Exit Power IDEA?",
                    &programs,
                    move |_, cx| quit(true, cx),
                    window,
                    cx,
                );
            });
            if shown.is_err() {
                quit(true, cx);
            }
        });
    })
    .detach();
}

impl Workspace {
    fn terminal_probe(&self, cx: &App) -> Probe {
        let mut probe = Probe::default();
        for terminal in &self.terminals {
            let terminal = terminal.read(cx);
            match terminal.run_state() {
                Some(run) if run.exit.is_none() => probe.runs.push(run.name.clone()),
                Some(_) => {}
                None => probe.shells.push(terminal.shell_pid()),
            }
        }
        probe
    }

    /// `Some(asked)` when the window may go on closing: nothing runs in its terminals,
    /// or the user agreed to stop it (`asked`). `None` while it checks in the
    /// background; it asks if needed and then closes the window itself.
    pub(super) fn terminals_allow_close(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<bool> {
        if let Some(asked) = self.terminals_checked.take() {
            return Some(asked);
        }
        let probe = self.terminal_probe(cx);
        if probe.is_empty() {
            return Some(false);
        }
        let programs = cx.background_spawn(async move { probe.programs() });
        cx.spawn_in(window, async move |this, cx| {
            let programs = programs.await;
            this.update_in(cx, |ws, window, cx| {
                if programs.is_empty() {
                    return ws.close_after_terminal_check(false, window, cx);
                }
                let workspace = cx.entity().downgrade();
                ask(
                    "Close project?",
                    &programs,
                    move |window, cx| {
                        workspace
                            .update(cx, |ws, cx| ws.close_after_terminal_check(true, window, cx))
                            .ok();
                    },
                    window,
                    cx,
                );
            })
            .ok();
        })
        .detach();
        None
    }

    fn close_after_terminal_check(&mut self, asked: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.terminals_checked = Some(asked);
        if self.should_close_window(window, cx) {
            window.remove_window();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::describe;

    #[test]
    fn lists_up_to_three_programs() {
        let names = |n: usize| (1..=n).map(|i| format!("p{i}")).collect::<Vec<_>>();
        assert_eq!(
            describe(&names(2)),
            "Running in the terminal: p1, p2. Closing will stop it."
        );
        assert_eq!(
            describe(&names(5)),
            "Running in the terminal: p1, p2, p3 and 2 more. Closing will stop it."
        );
    }
}
