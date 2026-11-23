//! Bottom terminal panel: tabs of shell sessions (Alt+F12).

use std::path::PathBuf;

use gpui_kit::component::{ActiveTheme as _, Icon, IconName, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{BottomPanel, Workspace};
use crate::config::AppConfig;
use crate::style;
use crate::terminal::{RunExit, Session, TerminalEvent, TerminalView};

impl Workspace {
    pub(super) fn terminal_focused(&self, window: &Window, cx: &App) -> bool {
        self.terminals
            .iter()
            .any(|t| t.read(cx).focus_handle().is_focused(window))
    }

    pub(super) fn terminal_visible(&self) -> bool {
        self.show_bottom && self.bottom_panel == BottomPanel::Terminal
    }

    fn show_terminal_panel(&mut self) {
        self.show_bottom = true;
        self.bottom_panel = BottomPanel::Terminal;
    }

    pub(super) fn focus_terminal(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(terminal) = self.terminals.get(self.active_terminal) {
            terminal.read(cx).focus_handle().clone().focus(window, cx);
        }
    }

    pub(super) fn new_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.new_terminal_in(self.root.clone(), window, cx);
    }

    /// Opens a terminal in the project root with `command` typed at the prompt.
    pub(super) fn open_terminal_with_input(
        &mut self,
        command: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.new_terminal(window, cx);
        if let Some(terminal) = self.terminals.last() {
            terminal.read(cx).type_text(command);
        }
    }

    /// Opens a new terminal tab whose shell starts in `cwd`.
    pub(super) fn new_terminal_in(&mut self, cwd: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let shell =
            Some(cx.global::<AppConfig>().0.terminal_shell.trim().to_string()).filter(|s| !s.is_empty());
        let session = match Session::spawn(cwd.clone(), shell) {
            Ok(session) => session,
            Err(err) => return self.notify_error(format!("Cannot start terminal: {err:#}"), window, cx),
        };
        let root = self.root.clone();
        let terminal = cx.new(|cx| TerminalView::new(session, cx).with_links(root, cwd));
        self.add_terminal(terminal, window, cx);
    }

    /// Adds a terminal tab, shows the panel and focuses it.
    pub(super) fn add_terminal(
        &mut self,
        terminal: Entity<TerminalView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.insert_terminal(self.terminals.len(), terminal, window, cx);
    }

    /// Inserts a terminal tab at `ix` (the Claude tab goes first), shows the panel and focuses it.
    // reason: `_`-prefixed fields keep a task/subscription alive; reassigning them is the intent.
    #[allow(clippy::used_underscore_binding)]
    pub(super) fn insert_terminal(
        &mut self,
        ix: usize,
        terminal: Entity<TerminalView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let subscription = cx.subscribe_in(
            &terminal,
            window,
            |this, terminal, event, window, cx| match event {
                TerminalEvent::TitleChanged | TerminalEvent::Finished => cx.notify(),
                TerminalEvent::Exited => {
                    if let Some(ix) = this.terminals.iter().position(|t| t == terminal) {
                        this.close_terminal(ix, window, cx);
                    }
                }
                TerminalEvent::OpenLocation { path, line, column } => {
                    let (path, line, column) =
                        (path.clone(), line.saturating_sub(1), column.saturating_sub(1));
                    this.jump_to(&path, line, column, window, cx);
                }
            },
        );
        self._subscriptions.push(subscription);
        let ix = ix.min(self.terminals.len());
        self.terminals.insert(ix, terminal);
        self.active_terminal = ix;
        self.show_terminal_panel();
        self.focus_terminal(window, cx);
        cx.notify();
    }

    pub(super) fn close_terminal(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.terminals.len() {
            return;
        }
        let had_focus = self.terminal_focused(window, cx);
        self.terminals.remove(ix);
        if self.terminals.is_empty() {
            if self.bottom_panel == BottomPanel::Terminal {
                self.show_bottom = false;
            }
            self.active_terminal = 0;
            self.focus_editor(window, cx);
        } else {
            self.active_terminal = self.active_terminal.min(self.terminals.len() - 1);
            if had_focus {
                self.focus_terminal(window, cx);
            }
        }
        cx.notify();
    }

    /// Alt+F12, JetBrains-style: open and focus, or hide when already focused.
    pub(super) fn toggle_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.wants_claude_tab(cx) && !self.has_claude_tab(cx) {
            return self.start_claude_tab(window, cx);
        }
        if self.terminals.is_empty() {
            return self.new_terminal(window, cx);
        }
        if self.terminal_visible() && self.terminal_focused(window, cx) {
            self.show_bottom = false;
            self.focus_editor(window, cx);
        } else {
            self.show_terminal_panel();
            self.focus_terminal(window, cx);
        }
        cx.notify();
    }

    pub(super) fn render_terminal_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let tabs = self.terminals.iter().enumerate().map(|(ix, terminal)| {
            let active = ix == self.active_terminal;
            h_flex()
                .id(("terminal-tab", ix))
                .h_full()
                .px_2()
                .gap_1()
                .text_xs()
                .border_b_2()
                .border_color(if active {
                    theme.primary
                } else {
                    transparent_black()
                })
                .when(active, |el| el.shadow(style::glow(theme.primary, 0.4)))
                .text_color(if active {
                    theme.foreground
                } else {
                    theme.muted_foreground
                })
                .when(terminal.read(cx).is_claude(), |el| {
                    el.child(Icon::new(IconName::Bot).size_3().text_color(theme.primary))
                })
                .child(terminal_label(terminal.read(cx)))
                .when(terminal.read(cx).is_claude(), |el| {
                    el.children(self.claude_tab_gauges(cx)).child(
                        style::icon_button_sized(
                            "restart-claude",
                            style::lucide("rotate-ccw"),
                            "Restart Claude",
                            style::BUTTON_SMALL,
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            cx.stop_propagation();
                            this.restart_claude_tab(window, cx);
                        })),
                    )
                })
                .child(
                    style::icon_button(("close-terminal", ix), IconName::Close, "Close (⌘W)").on_click(
                        cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.close_terminal_asking(ix, window, cx);
                        }),
                    ),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.active_terminal = ix;
                    this.focus_terminal(window, cx);
                    cx.notify();
                }))
        });

        v_flex()
            .size_full()
            .border_t_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .h(px(30.))
                    .px_2()
                    .gap_1()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .px_1()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child("Terminal"),
                    )
                    .children(tabs)
                    .child(
                        style::icon_button("new-terminal", IconName::Plus, "New tab (⌘T)")
                            .on_click(cx.listener(|this, _, window, cx| this.new_terminal(window, cx))),
                    )
                    .child(div().flex_1())
                    .child(
                        style::icon_button("hide-terminal", IconName::Minus, "Hide (⌥F12)").on_click(
                            cx.listener(|this, _, window, cx| {
                                this.show_bottom = false;
                                this.focus_editor(window, cx);
                                cx.notify();
                            }),
                        ),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .children(self.terminals.get(self.active_terminal).cloned()),
            )
    }
}

/// Tab label; run tabs show whether their process is running or how it ended.
fn terminal_label(terminal: &TerminalView) -> String {
    if terminal.is_claude() {
        return if terminal.is_running() {
            "Claude".into()
        } else {
            "Claude (exited)".into()
        };
    }
    let Some(run) = terminal.run_state() else {
        return terminal.title.clone();
    };
    let mark = match run.exit {
        None => "▶",
        Some(RunExit::Code(Some(0))) => "✓",
        Some(RunExit::Stopped) => "■",
        Some(RunExit::Code(_)) => "✗",
    };
    format!("{mark} {}", run.name)
}
