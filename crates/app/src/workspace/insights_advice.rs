//! "Ask Claude for advice" in the Insights panel: after the user confirms what
//! is sent, recent metrics and prompts go to the local `claude` CLI off the UI
//! thread, and the answer is shown under the tips.

use anyhow::Context as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::*;

use super::Workspace;
use crate::style;

#[derive(Default)]
pub(super) enum Advice {
    #[default]
    None,
    Asking,
    Ready(String),
    Failed(String),
}

impl Workspace {
    /// Asks first: the request carries prompt text, which leaves the machine.
    fn confirm_insights_advice(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let workspace = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |dialog, _, cx| {
            let workspace = workspace.clone();
            dialog
                .confirm()
                .width(px(440.))
                .icon(style::lucide("sparkles").text_color(cx.theme().primary))
                .title("Ask Claude for advice?")
                .child(format!(
                    "The numbers of your last {} sessions and their first prompts (shortened) are \
                     sent to Claude through the claude CLI. Nothing is saved as a Claude Code session.",
                    insights::advice::SESSIONS
                ))
                .footer(crate::style::confirm_buttons(
                    "Ask Claude",
                    gpui_kit::component::button::ButtonVariant::Primary,
                ))
                .on_ok(move |_, _, cx| {
                    workspace.update(cx, super::Workspace::ask_insights_advice).ok();
                    true
                })
        });
    }

    fn ask_insights_advice(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.insights.as_mut() else {
            return;
        };
        let sessions = state.scoped_sessions();
        state.advice = Advice::Asking;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let answer = cx
                .background_spawn(async move {
                    let claude = dirs::home_dir().context("no home directory")?.join(".claude");
                    let samples = insights::advice::samples(&claude, &sessions);
                    insights::advice::ask_claude(&insights::advice::request(&sessions, &samples))
                })
                .await;
            this.update(cx, |ws, cx| {
                let Some(state) = ws.insights.as_mut() else { return };
                state.advice = match answer {
                    Ok(text) => Advice::Ready(text),
                    Err(err) => Advice::Failed(format!("{err:#}")),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn render_insights_advice(&self, advice: &Advice, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (label, asking) = match advice {
            Advice::Asking => ("Asking Claude…", true),
            Advice::None => ("Ask Claude for advice", false),
            Advice::Ready(_) | Advice::Failed(_) => ("Ask again", false),
        };
        let body = match advice {
            Advice::Ready(text) => Some(div().child(text.clone())),
            Advice::Failed(err) => Some(div().text_color(theme.danger).child(err.clone())),
            Advice::None | Advice::Asking => None,
        };
        v_flex()
            .mx_2()
            .mb_1()
            .gap_1()
            .text_xs()
            .child(
                h_flex().child(
                    Button::new("insights-ask-claude")
                        .small()
                        .icon(style::lucide("sparkles"))
                        .label(label)
                        .loading(asking)
                        .disabled(asking)
                        .on_click(cx.listener(|ws, _, window, cx| ws.confirm_insights_advice(window, cx))),
                ),
            )
            .children(body.map(|body| {
                body.p_2()
                    .rounded(theme.radius)
                    .bg(theme.secondary)
                    .whitespace_normal()
            }))
            .into_any_element()
    }
}
