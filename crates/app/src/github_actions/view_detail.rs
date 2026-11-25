//! The run detail pane: run header, then jobs → steps → log lines in one virtualized list,
//! with error lines highlighted, search matches marked and live durations.

use std::collections::HashSet;
use std::ops::Range;
use std::rc::Rc;

use ci::time::{format_ago, format_duration, now_millis};
use ci::{LineKind, Run, State};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::rows::{self, DetailRow, LogState, Note};
use super::view::{state_color, state_icon};
use super::{CiView, Detail, Pane};
use crate::style;

const ROW_HEIGHT: Pixels = px(24.);
const STEP_INDENT: Pixels = px(28.);
const LOG_INDENT: Pixels = px(44.);

/// What every row of one frame needs, computed once per render.
struct Frame {
    rows: Vec<DetailRow>,
    matches: HashSet<usize>,
    current_match: Option<usize>,
    now: i64,
}

impl CiView {
    pub(super) fn render_detail(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let (Some(detail), Some(run)) = (&self.detail, self.selected_run()) else {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child("Select a run")
                .into_any_element();
        };
        let header = self.render_run_header(run, cx);
        let body = if !detail.loaded {
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .child(Spinner::new().small())
                .into_any_element()
        } else if let Some(error) = &detail.error {
            div()
                .p_4()
                .text_xs()
                .text_color(theme.danger)
                .child(error.clone())
                .into_any_element()
        } else {
            self.render_detail_rows(detail, cx)
        };
        v_flex()
            .size_full()
            .child(header)
            .child(div().flex_1().overflow_hidden().child(body))
            .into_any_element()
    }

    fn render_run_header(&self, run: &Run, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme();
        let now = now_millis();
        let state = run.state();
        let muted = theme.muted_foreground;
        let started = run.created_millis().map_or_default(|t| format_ago(now - t));
        let duration = run.duration_millis(now).map_or_default(format_duration);
        v_flex()
            .flex_none()
            .gap_1()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .gap_2()
                    .child(state_icon(state, cx))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(run.commit_subject().to_string()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(state_color(state, theme))
                            .child(state.label()),
                    ),
            )
            .child(
                h_flex()
                    .gap_1p5()
                    .text_xs()
                    .text_color(muted)
                    .child(format!("{} #{}", run.workflow_name(), run.run_number))
                    .when(run.run_attempt > 1, |el| {
                        el.child(format!("(attempt {})", run.run_attempt))
                    })
                    .child("·")
                    .child(run.event.clone())
                    .child("·")
                    .child(run.head_branch.clone().unwrap_or_default())
                    .child("·")
                    .child(
                        div()
                            .font_family(theme.mono_font_family.clone())
                            .child(run.short_sha().to_string()),
                    )
                    .children(run.actor.as_ref().map(|a| format!("· {}", a.login)))
                    .child("·")
                    .child(started)
                    .child("·")
                    .child(duration),
            )
    }

    fn render_detail_rows(&self, detail: &Detail, cx: &mut Context<Self>) -> AnyElement {
        let rows = detail.rows();
        let matches = self.search_matches(cx);
        let current_match = matches
            .get(self.search_ix.min(matches.len().saturating_sub(1)))
            .copied();
        let count = rows.len();
        let frame = Rc::new(Frame {
            rows,
            matches: matches.into_iter().collect(),
            current_match,
            now: now_millis(),
        });
        uniform_list(
            "ci-detail",
            count,
            cx.processor(move |view, range: Range<usize>, _, cx| {
                range
                    .filter_map(|ix| {
                        Some(view.render_detail_row(ix, frame.rows.get(ix)?.clone(), &frame, cx))
                    })
                    .collect()
            }),
        )
        .track_scroll(&self.detail_scroll)
        .size_full()
        .into_any_element()
    }

    fn render_detail_row(
        &self,
        ix: usize,
        row: DetailRow,
        frame: &Frame,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(detail) = &self.detail else {
            return div().into_any_element();
        };
        let theme = cx.theme();
        let at_cursor = self.pane == Pane::Detail && detail.cursor == Some(ix);
        let base = h_flex()
            .id(("ci-row", ix))
            .h(ROW_HEIGHT)
            .w_full()
            .pr_3()
            .gap_1p5()
            .when(at_cursor, |el| el.bg(theme.list_active));
        let chevron = |open: bool| {
            component::Icon::new(if open {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            })
            .with_size(style::TREE_CHEVRON)
            .text_color(theme.muted_foreground)
        };
        match row {
            DetailRow::Job { job } => {
                let Some(j) = detail.jobs.get(job) else {
                    return base.into_any_element();
                };
                let open = detail.expansion.jobs.contains(&j.id);
                let duration = j.duration_millis(frame.now).map_or_default(format_duration);
                base.pl_2()
                    .cursor_pointer()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .hover(|s| s.bg(theme.list_hover))
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.focus.focus(window, cx);
                        view.set_cursor(ix, cx);
                        view.toggle_job(job, cx);
                    }))
                    .child(chevron(open))
                    .child(state_icon(j.state(), cx))
                    .child(div().min_w_0().truncate().child(j.name.clone()))
                    .child(div().flex_1())
                    .child(div().text_xs().text_color(theme.muted_foreground).child(duration))
                    .into_any_element()
            }
            DetailRow::Annotation { job, ix: a } => {
                let Some(annotation) = detail
                    .jobs
                    .get(job)
                    .and_then(|j| detail.annotations.get(&j.id)?.get(a))
                else {
                    return base.into_any_element();
                };
                let color = if annotation.annotation_level == "failure" {
                    theme.danger
                } else {
                    theme.warning
                };
                let location = format!("{}:{}", annotation.path, annotation.start_line);
                let message = annotation.message.lines().next().unwrap_or_default().to_string();
                base.pl(STEP_INDENT)
                    .cursor_pointer()
                    .text_xs()
                    .hover(|s| s.bg(theme.list_hover))
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.set_cursor(ix, cx);
                        view.open_annotation(job, a, window, cx);
                    }))
                    .child(style::lucide("circle-alert").xsmall().text_color(color))
                    .child(
                        div()
                            .flex_none()
                            .text_color(theme.primary)
                            .font_family(theme.mono_font_family.clone())
                            .child(location),
                    )
                    .child(div().min_w_0().truncate().text_color(color).child(message))
                    .into_any_element()
            }
            DetailRow::Step { job, step } => {
                let Some((j, s)) = detail.jobs.get(job).and_then(|j| Some((j, j.steps.get(step)?))) else {
                    return base.into_any_element();
                };
                let open = detail.expansion.steps.contains(&(j.id, s.number));
                let state = s.state();
                let duration = s.duration_millis(frame.now).map_or_default(format_duration);
                let has_log = matches!(detail.logs.get(&j.id), Some(LogState::Ready(_)));
                base.pl(STEP_INDENT)
                    .cursor_pointer()
                    .text_sm()
                    .group("ci-step")
                    .when(state == State::Running, |el| el.bg(theme.warning.opacity(0.08)))
                    .when(state == State::Failure, |el| el.text_color(theme.danger))
                    .hover(|s| s.bg(theme.list_hover))
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.focus.focus(window, cx);
                        view.set_cursor(ix, cx);
                        view.toggle_step(job, step, cx);
                    }))
                    .child(chevron(open))
                    .child(state_icon(state, cx))
                    .child(div().min_w_0().truncate().child(s.name.clone()))
                    .child(div().flex_1())
                    .when(has_log, |el| {
                        el.child(
                            div().invisible().group_hover("ci-step", Styled::visible).child(
                                style::icon_button(("ci-copy-step", ix), IconName::Copy, "Copy step log")
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        cx.stop_propagation();
                                        view.copy_step(job, step, cx);
                                    })),
                            ),
                        )
                    })
                    .child(div().text_xs().text_color(theme.muted_foreground).child(duration))
                    .into_any_element()
            }
            DetailRow::Log { job, line } => self.render_log_line(ix, job, line, frame, cx),
            DetailRow::Note { note, .. } => {
                let (text, spinner) = match note {
                    Note::Loading => ("Loading the log…".to_string(), true),
                    Note::StillRunning => (
                        "The log is available when the job finishes; steps above show progress.".into(),
                        false,
                    ),
                    Note::NoOutput => ("No output".into(), false),
                    Note::Failed(message) => (message, false),
                };
                base.pl(LOG_INDENT)
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .when(spinner, |el| el.child(Spinner::new().xsmall()))
                    .child(text)
                    .into_any_element()
            }
        }
    }

    fn render_log_line(
        &self,
        ix: usize,
        job: usize,
        line: usize,
        frame: &Frame,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let Some(detail) = &self.detail else {
            return div().into_any_element();
        };
        let Some(LogState::Ready(log)) = detail.jobs.get(job).and_then(|j| detail.logs.get(&j.id)) else {
            return div().into_any_element();
        };
        let Some(entry) = log.lines.get(line) else {
            return div().into_any_element();
        };
        let start = entry
            .step
            .and_then(|step| rows::step_lines(log, step))
            .map_or(0, |r| r.start);
        let selected = detail
            .selection
            .is_some_and(|(j, a, b)| j == job && (a.min(b)..=a.max(b)).contains(&line));
        let (color, background) = match entry.kind {
            LineKind::Error => (theme.danger, Some(theme.danger.opacity(0.1))),
            LineKind::Warning => (theme.warning, Some(theme.warning.opacity(0.08))),
            LineKind::Command => (theme.info, None),
            LineKind::Debug => (theme.muted_foreground, None),
            LineKind::Notice | LineKind::Group | LineKind::Normal => (theme.foreground, None),
        };
        let background = if frame.current_match == Some(ix) {
            Some(theme.warning.opacity(0.35))
        } else if frame.matches.contains(&ix) {
            Some(theme.warning.opacity(0.15))
        } else if selected {
            Some(theme.primary.opacity(0.18))
        } else {
            background
        };
        let indent = px(f32::from(entry.depth) * 12.);
        h_flex()
            .id(("ci-log", ix))
            .h(ROW_HEIGHT)
            .w_full()
            .pl(LOG_INDENT)
            .pr_3()
            .gap_3()
            .font_family(theme.mono_font_family.clone())
            .text_xs()
            .when_some(background, Styled::bg)
            .on_click(cx.listener(move |view, event: &ClickEvent, window, cx| {
                view.focus.focus(window, cx);
                view.click_log_line(job, line, event.modifiers().shift, cx);
            }))
            .child(
                div()
                    .w(px(40.))
                    .flex_none()
                    .text_right()
                    .text_color(theme.muted_foreground.opacity(0.6))
                    .child((line - start + 1).to_string()),
            )
            .child(
                div()
                    .pl(indent)
                    .min_w_0()
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .text_color(color)
                    .when(entry.kind == LineKind::Group, |el| {
                        el.font_weight(FontWeight::SEMIBOLD)
                    })
                    .child(entry.text.clone()),
            )
            .into_any_element()
    }
}
