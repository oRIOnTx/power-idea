//! Small pieces of the Insights panel: formatting helpers, block bars and
//! the selected session's breakdown.

use std::time::{SystemTime, UNIX_EPOCH};

use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use insights::{Block, SessionSummary};

pub(super) fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

pub(super) fn folder_name(path: &str) -> String {
    path.rsplit('/')
        .find(|p| !p.is_empty())
        .unwrap_or(path)
        .to_string()
}

pub(super) fn tokens_label(tokens: u64) -> String {
    match tokens {
        0..1_000 => tokens.to_string(),
        1_000..1_000_000 => format!("{:.0}K", tokens as f64 / 1e3),
        _ => format!("{:.1}M", tokens as f64 / 1e6),
    }
}

pub(super) fn duration_label(secs: i64) -> String {
    match secs {
        ..60 => format!("{secs}s"),
        60..3600 => format!("{}m", secs / 60),
        _ => format!("{}h {}m", secs / 3600, secs % 3600 / 60),
    }
}

pub(super) fn score_color(score: f64, cx: &App) -> Hsla {
    let theme = cx.theme();
    match score {
        s if s >= 75.0 => theme.green,
        s if s >= 50.0 => theme.yellow,
        _ => theme.danger,
    }
}

pub(super) fn section(title: &'static str, cx: &App) -> Div {
    div()
        .px_3()
        .pt_3()
        .pb_1()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(cx.theme().muted_foreground)
        .child(title.to_uppercase())
}

/// Seven horizontal bars; a thin marker shows the previous period's value.
pub(super) fn render_blocks(now: Option<[f64; 7]>, before: Option<[f64; 7]>, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let Some(now) = now else {
        return div()
            .px_3()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child("No sessions in this period")
            .into_any_element();
    };
    v_flex()
        .px_3()
        .gap_1()
        .children(Block::ALL.iter().enumerate().map(|(ix, block)| {
            let value = now[ix];
            let previous = before.map(|b| b[ix]);
            h_flex()
                .gap_2()
                .text_xs()
                .child(
                    div()
                        .w(px(92.))
                        .flex_none()
                        .text_color(theme.muted_foreground)
                        .child(block.label()),
                )
                .child(
                    div()
                        .relative()
                        .flex_1()
                        .h(px(6.))
                        .rounded_full()
                        .bg(theme.secondary)
                        .child(
                            div()
                                .h_full()
                                .rounded_full()
                                .w(relative((value / 100.0) as f32))
                                .bg(score_color(value, cx)),
                        )
                        .when_some(previous, |el, previous| {
                            el.child(
                                div()
                                    .absolute()
                                    .top(px(-2.))
                                    .bottom(px(-2.))
                                    .left(relative((previous / 100.0) as f32))
                                    .w(px(2.))
                                    .bg(theme.foreground.opacity(0.6)),
                            )
                        }),
                )
                .child(
                    div()
                        .w(px(22.))
                        .flex_none()
                        .text_right()
                        .child(format!("{value:.0}")),
                )
        }))
        .into_any_element()
}

pub(super) fn render_session_details(session: &SessionSummary, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let m = &session.metrics;
    let yes_no = |flag: bool| if flag { "yes" } else { "no" };
    let facts = [
        (
            "Prompts",
            format!("{} ({} corrections)", m.prompts, m.corrections),
        ),
        (
            "Tool calls",
            format!("{} ({} failed, {} loops)", m.tool_calls, m.tool_errors, m.loops),
        ),
        ("Steps / prompt", format!("{:.1}", m.steps_per_prompt)),
        ("Interrupts", m.interrupts.to_string()),
        (
            "Verified · committed",
            format!("{} · {}", yes_no(m.verified), yes_no(m.committed)),
        ),
        (
            "Tokens · cache hit",
            format!("{} · {:.0}%", tokens_label(m.total_tokens), m.cache_hit * 100.0),
        ),
        ("Compactions", m.compactions.to_string()),
        ("Model", session.model.clone()),
    ];
    v_flex()
        .mx_3()
        .mt_2()
        .p_2()
        .gap_0p5()
        .rounded(theme.radius)
        .border_1()
        .border_color(theme.border)
        .text_xs()
        .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(format!(
            "{} · {}",
            folder_name(&session.project),
            insights::day_label(session.started, now())
        )))
        .children(facts.into_iter().map(|(label, value)| {
            h_flex()
                .gap_2()
                .child(
                    div()
                        .w(px(118.))
                        .flex_none()
                        .text_color(theme.muted_foreground)
                        .child(label),
                )
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(value),
                )
        }))
        .into_any_element()
}
