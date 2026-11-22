//! Insights panel rendering: one score with its trend, a score-per-session
//! chart, block bars (now vs previous period), up to three tips, the session
//! list and the selected session's breakdown. State lives in `insights_panel`.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, Selectable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use insights::{Insight, Period, Report, SessionSummary};

use super::Workspace;
use super::insights_panel::{InsightsState, InsightsStatus};
use crate::style;

use super::insights_parts::{
    duration_label, folder_name, now, render_blocks, render_session_details, score_color, section,
    tokens_label,
};

const CHART_HEIGHT: f32 = 56.;
/// Sessions drawn in the chart (the most recent ones).
const CHART_SESSIONS: usize = 40;

impl Workspace {
    pub(super) fn render_insights_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(state) = &self.insights else {
            return div().into_any_element();
        };
        let message = |text: String| {
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .p_4()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(style::lucide("chart-line").size_6())
                .child(div().text_center().child(text))
                .into_any_element()
        };
        match &state.status {
            InsightsStatus::Indexing => return message("Reading Claude Code sessions…".into()),
            InsightsStatus::Failed(err) => return message(err.clone()),
            InsightsStatus::Ready if state.sessions.is_empty() => {
                return message("No Claude Code sessions in ~/.claude/projects yet.".into());
            }
            InsightsStatus::Ready => {}
        }
        let report = insights::report(&state.sessions, state.period, state.project.as_deref(), now());
        let selected = state
            .selected
            .as_ref()
            .and_then(|id| state.sessions.iter().find(|s| &s.id == id))
            .cloned();
        let tips = state.tips();

        v_flex()
            .id("insights")
            .size_full()
            .overflow_y_scroll()
            .pb_3()
            .child(self.render_insights_header(state, &report, cx))
            .child(self.render_insights_chart(&report, selected.as_ref(), cx))
            .child(section("Blocks", cx))
            .child(render_blocks(
                selected.as_ref().map_or(report.blocks_now, |s| Some(s.blocks)),
                if selected.is_some() {
                    None
                } else {
                    report.blocks_before
                },
                cx,
            ))
            .when_some(selected, |el, session| {
                el.child(render_session_details(&session, cx))
            })
            .when(!tips.is_empty(), |el| {
                el.child(section("Tips", cx)).children(
                    tips.into_iter()
                        .enumerate()
                        .map(|(ix, tip)| self.render_tip(ix, tip, cx)),
                )
            })
            .child(section("Sessions", cx))
            .children(
                report
                    .sessions
                    .iter()
                    .rev()
                    .enumerate()
                    .map(|(ix, s)| self.render_session_row(ix, s, state, cx)),
            )
            .into_any_element()
    }

    fn render_insights_header(
        &self,
        state: &InsightsState,
        report: &Report,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let score = report.score;
        let delta = report.delta;
        let project_label = state
            .project
            .as_deref()
            .map_or_else(|| "All projects".to_string(), folder_name);
        let projects = insights::projects(&state.sessions);
        let current_project = state.project.clone();
        let workspace = cx.entity().downgrade();

        let periods = h_flex().gap_0p5().children(Period::ALL.map(|period| {
            Button::new(SharedString::from(format!("insights-period-{}", period.label())))
                .ghost()
                .xsmall()
                .label(period.label())
                .selected(state.period == period)
                .on_click(cx.listener(move |ws, _, _, cx| {
                    if let Some(state) = ws.insights.as_mut() {
                        state.period = period;
                    }
                    cx.notify();
                }))
        }));

        let project_menu =
            Button::new("insights-project")
                .ghost()
                .xsmall()
                .label(project_label)
                .dropdown_caret(true)
                .dropdown_menu(move |mut menu, _, _| {
                    let choices = std::iter::once(None).chain(projects.iter().cloned().map(Some));
                    for choice in choices {
                        let label = choice
                            .as_deref()
                            .map_or_else(|| "All projects".to_string(), folder_name);
                        let checked = choice == current_project;
                        let workspace = workspace.clone();
                        menu = menu.item(PopupMenuItem::new(label).checked(checked).on_click(
                            move |_, _, cx| {
                                workspace
                                    .update(cx, |ws, cx| {
                                        if let Some(state) = ws.insights.as_mut() {
                                            state.project = choice.clone();
                                            state.selected = None;
                                        }
                                        cx.notify();
                                    })
                                    .ok();
                            },
                        ));
                    }
                    menu
                });

        v_flex()
            .px_3()
            .pt_3()
            .gap_2()
            .child(
                h_flex()
                    .items_end()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(34.))
                            .line_height(px(36.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(score.map_or(theme.muted_foreground, |s| score_color(s, cx)))
                            .child(score.map_or_else(|| "—".to_string(), |s| format!("{s:.0}"))),
                    )
                    .child(
                        v_flex()
                            .pb_0p5()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child("Session Score"),
                            )
                            .child(match delta {
                                Some(d) => div()
                                    .text_xs()
                                    .text_color(if d >= 0.0 { theme.green } else { theme.danger })
                                    .child(format!(
                                        "{} {:+.0} vs previous",
                                        if d >= 0.0 { "↑" } else { "↓" },
                                        d
                                    )),
                                None => div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child("no earlier data"),
                            }),
                    )
                    .child(div().flex_1())
                    .child(
                        style::icon_button_sized(
                            "insights-refresh",
                            style::lucide("refresh-cw"),
                            "Re-index sessions",
                            style::BUTTON_SMALL,
                        )
                        .on_click(cx.listener(|ws, _, _, cx| ws.reindex_insights(cx))),
                    ),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child(periods)
                    .child(div().flex_1())
                    .child(project_menu),
            )
            .child(div().text_xs().text_color(theme.muted_foreground).child(format!(
                "{} sessions · {} tokens · ${:.2}",
                report.sessions.len(),
                tokens_label(report.total_tokens),
                report.total_cost
            )))
            .into_any_element()
    }

    /// One bar per session (height = score); click selects the session.
    fn render_insights_chart(
        &self,
        report: &Report,
        selected: Option<&SessionSummary>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let recent = &report.sessions[report.sessions.len().saturating_sub(CHART_SESSIONS)..];
        let bars = recent.iter().enumerate().map(|(ix, session)| {
            let id = session.id.clone();
            let is_selected = selected.is_some_and(|s| s.id == session.id);
            let color = score_color(session.score, cx);
            div()
                .id(("insights-bar", ix))
                .flex_1()
                .max_w(px(10.))
                .h_full()
                .flex()
                .items_end()
                .cursor_pointer()
                .child(
                    div()
                        .w_full()
                        .h(px((session.score as f32 / 100.0 * CHART_HEIGHT).max(2.)))
                        .rounded_t(px(2.))
                        .bg(if is_selected {
                            theme.primary
                        } else {
                            color.opacity(0.75)
                        }),
                )
                .hover(|s| s.opacity(0.8))
                .on_click(cx.listener(move |ws, _, _, cx| ws.select_insights_session(&id, cx)))
        });
        div()
            .mx_3()
            .mt_3()
            .h(px(CHART_HEIGHT))
            .flex()
            .items_end()
            .gap(px(2.))
            .border_b_1()
            .border_color(theme.border)
            .children(bars)
            .into_any_element()
    }

    fn select_insights_session(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(state) = self.insights.as_mut() {
            state.selected = (state.selected.as_deref() != Some(id)).then(|| id.to_string());
        }
        cx.notify();
    }

    fn render_tip(&self, ix: usize, tip: Insight, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let id = tip.id;
        v_flex()
            .mx_2()
            .mb_1()
            .p_2()
            .gap_1()
            .rounded(theme.radius)
            .bg(theme.secondary)
            .text_xs()
            .child(
                h_flex()
                    .gap_1()
                    .child(style::lucide("sparkles").size_3().text_color(theme.primary))
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(tip.title),
                    )
                    .child(
                        Button::new(("insights-dismiss", ix))
                            .ghost()
                            .xsmall()
                            .label("Got it")
                            .on_click(cx.listener(move |ws, _, _, cx| ws.dismiss_tip(id, cx))),
                    ),
            )
            .child(div().child(tip.evidence))
            .child(div().text_color(theme.muted_foreground).child(tip.why))
            .child(
                div()
                    .text_color(theme.primary)
                    .child(format!("Try: {}", tip.try_this)),
            )
            .into_any_element()
    }

    fn render_session_row(
        &self,
        ix: usize,
        session: &SessionSummary,
        state: &InsightsState,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let id = session.id.clone();
        let is_selected = state.selected.as_deref() == Some(&session.id);
        let m = &session.metrics;
        h_flex()
            .id(("insights-session", ix))
            .mx_1()
            .px_2()
            .py_1()
            .gap_2()
            .rounded(theme.radius)
            .cursor_pointer()
            .when(is_selected, |el| el.bg(theme.list_active))
            .hover(|s| s.bg(theme.list_hover))
            .on_click(cx.listener(move |ws, _, _, cx| ws.select_insights_session(&id, cx)))
            .child(
                div()
                    .flex_none()
                    .w(px(26.))
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(score_color(session.score, cx))
                    .child(format!("{:.0}", session.score)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_sm()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .child(folder_name(&session.project)),
                    )
                    .child(div().text_xs().text_color(theme.muted_foreground).child(format!(
                        "{} · {} · {} tok · ${:.2}",
                        insights::day_label(session.started, now()),
                        duration_label(m.duration_secs),
                        tokens_label(m.total_tokens),
                        m.cost_usd
                    ))),
            )
            .into_any_element()
    }
}
