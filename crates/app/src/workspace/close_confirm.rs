//! "Close project?" and "Exit Power IDEA?" confirmations (JetBrains "Confirm application
//! exit"); "Don't ask again" turns the matching setting off.

use std::rc::Rc;

use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::{ActiveTheme as _, WindowExt as _};
use gpui_kit::*;

use super::{Workspace, Workspaces};
use crate::config::{AppConfig, Config};
use crate::style;

struct Confirmation {
    title: String,
    message: &'static str,
    icon: &'static str,
    ok_text: &'static str,
    /// Turns the setting off when "Don't ask again" is checked.
    never_ask: fn(&mut Config),
}

fn ask(
    confirmation: Confirmation,
    on_ok: impl Fn(&mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let dont_ask = cx.new(|_| false);
    let on_ok = Rc::new(on_ok);
    window.open_alert_dialog(cx, move |dialog, _, cx| {
        let (dont_ask, toggle, on_ok) = (dont_ask.clone(), dont_ask.clone(), on_ok.clone());
        let never_ask = confirmation.never_ask;
        dialog
            .confirm()
            .width(px(420.))
            .icon(style::lucide(confirmation.icon).text_color(cx.theme().primary))
            .title(confirmation.title.clone())
            .description(confirmation.message)
            .child(
                Checkbox::new("dont-ask-again")
                    .label("Don't ask again")
                    .checked(*dont_ask.read(cx))
                    .on_click(move |checked: &bool, _, cx| {
                        toggle.update(cx, |value, cx| {
                            *value = *checked;
                            cx.notify();
                        });
                    }),
            )
            .footer(style::confirm_buttons(
                confirmation.ok_text,
                component::button::ButtonVariant::Primary,
            ))
            .on_ok(move |_, window, cx| {
                if *dont_ask.read(cx) {
                    never_ask(&mut cx.global_mut::<AppConfig>().0);
                    crate::save_config(cx);
                }
                on_ok(window, cx);
                true
            })
    });
}

/// ⌘Q with nothing unsaved: asks in the active project window, then quits.
/// `confirmed`: the user already agreed to exit (running terminals dialog).
pub(super) fn confirm_quit(confirmed: bool, cx: &mut App) {
    if confirmed {
        return cx.quit();
    }
    let live = Workspaces::live(cx);
    let active = cx.active_window();
    let target = live
        .iter()
        .map(|(handle, _)| *handle)
        .find(|handle| Some(*handle) == active)
        .or_else(|| live.first().map(|(handle, _)| *handle));
    let Some(handle) = target.filter(|_| cx.global::<AppConfig>().0.confirm_exit) else {
        return cx.quit();
    };
    let message = match live.len() {
        1 => "The project window will be closed.",
        _ => "All project windows will be closed.",
    };
    let shown = handle.update(cx, |_, window, cx| {
        window.activate_window();
        let confirmation = Confirmation {
            title: "Exit Power IDEA?".into(),
            message,
            icon: "power",
            ok_text: "Exit",
            never_ask: |config| config.confirm_exit = false,
        };
        ask(confirmation, |_, cx| cx.quit(), window, cx);
    });
    if shown.is_err() {
        cx.quit();
    }
}

impl Workspace {
    /// ⇧⌘W, the window's close button and project tabs: asks about running terminals,
    /// unsaved files, then "Close project?". Returns true when the window can close right away.
    pub(crate) fn should_close_window(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(asked) = self.terminals_allow_close(window, cx) else {
            return false;
        };
        if !self.can_close_window(window, cx) {
            return false;
        }
        // "Terminate" in the running-terminals dialog already confirmed closing.
        if asked || !cx.global::<AppConfig>().0.confirm_close_project {
            return true;
        }
        let confirmation = Confirmation {
            title: format!("Close project “{}”?", self.project_name()),
            message: "Open files and tool windows are restored when you open it again.",
            icon: "log-out",
            ok_text: "Close",
            never_ask: |config| config.confirm_close_project = false,
        };
        ask(confirmation, |window, _| window.remove_window(), window, cx);
        false
    }
}
