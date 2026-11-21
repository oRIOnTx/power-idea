//! Bottom terminal panel: tabs of shell sessions (Alt+F12).

use std::path::PathBuf;

use gpui_kit::component::{ActiveTheme as _, IconName, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::Workspace;
use crate::config::AppConfig;
use crate::style;
use crate::terminal::{Session, TerminalEvent, TerminalView};

impl Workspace {
    pub(super) fn terminal_focused(&self, window: &Window, cx: &App) -> bool {
        self.terminals
            .iter()
            .any(|t| t.read(cx).focus_handle().is_focused(window))
    }

    pub(super) fn focus_terminal(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(terminal) = self.terminals.get(self.active_terminal) {
            terminal.read(cx).focus_handle().clone().focus(window, cx);
        }
    }

    pub(super) fn new_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.new_terminal_in(self.root.clone(), window, cx);
    }

    /// Opens a new terminal tab whose shell starts in `cwd`.
    pub(super) fn new_terminal_in(&mut self, cwd: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let shell =
            Some(cx.global::<AppConfig>().0.terminal_shell.trim().to_string()).filter(|s| !s.is_empty());
        let session = match Session::spawn(cwd, shell) {
            Ok(session) => session,
            Err(err) => return self.notify_error(format!("Cannot start terminal: {err:#}"), window, cx),
        };
        let terminal = cx.new(|cx| TerminalView::new(session, cx));
        let subscription = cx.subscribe_in(
            &terminal,
            window,
            |this, terminal, event, window, cx| match event {
                TerminalEvent::TitleChanged => cx.notify(),
                TerminalEvent::Exited => {
                    if let Some(ix) = this.terminals.iter().position(|t| t == terminal) {
                        this.close_terminal(ix, window, cx);
                    }
                }
            },
        );
        self._subscriptions.push(subscription);
        self.terminals.push(terminal);
        self.active_terminal = self.terminals.len() - 1;
        self.show_terminal = true;
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
            self.show_terminal = false;
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
        if self.terminals.is_empty() {
            return self.new_terminal(window, cx);
        }
        if self.show_terminal && self.terminal_focused(window, cx) {
            self.show_terminal = false;
            self.focus_editor(window, cx);
        } else {
            self.show_terminal = true;
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
                    gpui_kit::transparent_black()
                })
                .when(active, |el| el.shadow(style::glow(theme.primary, 0.4)))
                .text_color(if active {
                    theme.foreground
                } else {
                    theme.muted_foreground
                })
                .child(terminal.read(cx).title.clone())
                .child(
                    style::icon_button(("close-terminal", ix), IconName::Close, "Close (⌘W)").on_click(
                        cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.close_terminal(ix, window, cx);
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
                                this.show_terminal = false;
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
