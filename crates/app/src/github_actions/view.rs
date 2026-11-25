//! GitHub Actions tab layout: header with HEAD status, the setup states (no repo, not
//! GitHub, not connected), and the run list; the detail pane is in `view_detail`.

use ci::time::{format_ago, format_duration, now_millis};
use ci::{Run, State, TokenSource};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, Theme, h_flex, h_resizable,
    resizable_panel, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::claude::Deliver;
use super::fetch::Setup;
use super::run_actions::RunAction;
use super::{
    Back, CI_CONTEXT, CiView, CopySelection, FindInLog, OpenOnGitHub, OpenSelected, Pane, Refresh,
    RerunFailed, RunWorkflow, Scope, SendToClaude,
};
use crate::actions::{SelectNext, SelectPrev};
use crate::style;

const RUN_ROW_HEIGHT: Pixels = px(48.);

pub(super) fn state_color(state: State, theme: &Theme) -> Hsla {
    match state {
        State::Success => theme.success,
        State::Failure => theme.danger,
        State::Running => theme.warning,
        State::Queued | State::Cancelled | State::Skipped | State::Neutral => theme.muted_foreground,
    }
}

/// Every status icon sits in the same box, so a spinner and a static icon line text up alike.
pub(super) const STATE_ICON_BOX: Pixels = px(16.);

/// A run, job or step status: a spinner while running, else a colored icon.
pub(super) fn state_icon(state: State, cx: &App) -> AnyElement {
    let color = state_color(state, cx.theme());
    let slot = || {
        div()
            .flex_none()
            .size(STATE_ICON_BOX)
            .flex()
            .items_center()
            .justify_center()
    };
    let name = match state {
        State::Running => {
            return slot()
                .child(
                    Spinner::new()
                        .icon(style::lucide("loader-circle"))
                        .xsmall()
                        .color(color),
                )
                .into_any_element();
        }
        State::Success => "circle-check",
        State::Failure => "circle-x",
        State::Queued => "circle-dashed",
        State::Cancelled => "circle-slash",
        State::Skipped => "circle-minus",
        State::Neutral => "circle",
    };
    slot()
        .child(style::lucide(name).xsmall().text_color(color))
        .into_any_element()
}

impl CiView {
    fn render_header(&self, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme().clone();
        let repo = self.client.as_ref().map(|c| c.repo().to_string());
        let branch = self.branch.clone().unwrap_or_else(|| "detached HEAD".into());
        let head = self.head_state.map(|state| {
            let label = match state {
                State::Success => "passing",
                State::Failure => "failing",
                State::Running => "running",
                State::Queued => "queued",
                _ => state.label(),
            };
            h_flex()
                .gap_1()
                .px_2()
                .py_0p5()
                .rounded_md()
                .bg(state_color(state, &theme).opacity(0.12))
                .text_xs()
                .text_color(state_color(state, &theme))
                .child(state_icon(state, cx))
                .child(format!("HEAD {label}"))
        });
        let updated = self.last_updated.map(|t| {
            if self.fetching {
                "Updating…".to_string()
            } else {
                format!("Updated {}", format_ago(now_millis() - t))
            }
        });
        let scope_button = |id: &'static str, label: &'static str, scope: Scope, cx: &mut Context<Self>| {
            Button::new(id)
                .xsmall()
                .ghost()
                .label(label)
                .selected(self.scope == scope)
                .on_click(cx.listener(move |view, _, _, cx| view.set_scope(scope, cx)))
        };
        h_flex()
            .flex_none()
            .gap_2()
            .px_3()
            .h(px(40.))
            .border_b_1()
            .border_color(theme.border)
            .child(
                style::lucide("workflow")
                    .small()
                    .text_color(theme.muted_foreground),
            )
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(repo.unwrap_or_else(|| "GitHub Actions".into())),
            )
            .when(self.setup == Setup::Ready, |el| {
                el.child(
                    h_flex()
                        .gap_1()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(style::lucide("git-branch").xsmall())
                        .child(branch),
                )
                .children(head)
                .child(
                    h_flex()
                        .gap_0p5()
                        .child(scope_button("ci-scope-branch", "This branch", Scope::Branch, cx))
                        .child(scope_button("ci-scope-all", "All", Scope::All, cx)),
                )
                .child(
                    Button::new("ci-run-workflow")
                        .xsmall()
                        .ghost()
                        .icon(style::lucide("play"))
                        .label("Run workflow…")
                        .tooltip("Run workflow (D)")
                        .loading(self.busy == Some(super::run_actions::Busy::Dispatch))
                        .on_click(cx.listener(|view, _, window, cx| view.open_run_dialog(window, cx))),
                )
            })
            .child(div().flex_1())
            .when(self.setup == Setup::Ready, |el| {
                el.child(
                    div()
                        .w(px(220.))
                        .key_context("CiSearch")
                        .child(Input::new(&self.search).xsmall().cleanable(true)),
                )
            })
            .children(updated.map(|u| div().text_xs().text_color(theme.muted_foreground).child(u)))
            .when(self.setup == Setup::Ready, |el| {
                el.child(
                    style::icon_button_sized(
                        "ci-refresh",
                        style::lucide("refresh-cw"),
                        "Refresh (⌘R)",
                        style::BUTTON_SMALL,
                    )
                    .loading(self.fetching)
                    .on_click(cx.listener(|view, _, _, cx| view.refresh(cx))),
                )
            })
            .child(
                style::icon_button_sized(
                    "ci-open-github",
                    style::lucide("external-link"),
                    "Open on GitHub (O)",
                    style::BUTTON_SMALL,
                )
                .disabled(self.client.is_none())
                .on_click(cx.listener(|view, _, _, cx| view.open_on_github(cx))),
            )
            .when(self.token_source == Some(TokenSource::Keychain), |el| {
                el.child(
                    style::icon_button_sized(
                        "ci-forget-token",
                        style::lucide("key-round"),
                        "Forget the saved token",
                        style::BUTTON_SMALL,
                    )
                    .on_click(cx.listener(|view, _, _, cx| view.forget_pat(cx))),
                )
            })
    }

    fn render_runs(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let runs = self.shown_runs();
        if !self.runs_loaded && runs.is_empty() {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .child(Spinner::new().small())
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child("Loading runs…"),
                )
                .into_any_element();
        }
        if runs.is_empty() {
            let (title, hint) = match (self.scope, &self.branch) {
                (Scope::Branch, Some(branch)) => (
                    "No runs on this branch".to_string(),
                    format!("Nothing ran on {branch} yet (or it is not pushed)."),
                ),
                (Scope::Branch, None) => (
                    "Detached HEAD".to_string(),
                    "Runs are listed per branch; show all branches instead.".to_string(),
                ),
                (Scope::All, _) => (
                    "No workflow runs".to_string(),
                    "This repository has no runs yet.".to_string(),
                ),
            };
            let all = (self.scope == Scope::Branch).then(|| {
                Button::new("ci-show-all")
                    .small()
                    .label("Show all branches")
                    .on_click(cx.listener(|view, _, _, cx| view.set_scope(Scope::All, cx)))
                    .into_any_element()
            });
            return self
                .render_message("workflow", &title, hint, all.into_iter().collect(), cx)
                .into_any_element();
        }
        let count = runs.len();
        uniform_list(
            "ci-runs",
            count,
            cx.processor(|view, range: std::ops::Range<usize>, _, cx| {
                let now = now_millis();
                range
                    .filter_map(|ix| {
                        let run = view.shown_runs().get(ix)?.clone();
                        Some(view.render_run_row(ix, &run, now, cx))
                    })
                    .collect()
            }),
        )
        .track_scroll(&self.runs_scroll)
        .size_full()
        .into_any_element()
    }

    fn render_run_row(&self, ix: usize, run: &Run, now: i64, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let selected = self.selected_run == Some(run.id);
        let focused = selected && self.pane == Pane::Runs;
        let is_head = self.head_sha.as_deref() == Some(run.head_sha.as_str());
        let id = run.id;
        let state = run.state();
        let age = run.created_millis().map_or_default(|t| format_ago(now - t));
        let duration = run.duration_millis(now).map_or_default(format_duration);
        let muted = theme.muted_foreground;
        h_flex()
            .id(("ci-run", ix))
            .h(RUN_ROW_HEIGHT)
            .px_3()
            .gap_2()
            .relative()
            .cursor_pointer()
            .border_b_1()
            .border_color(theme.border.opacity(0.5))
            .when(selected, |el| el.bg(theme.list_active))
            .when(!selected, |el| el.hover(|s| s.bg(theme.list_hover)))
            .when(focused, |el| {
                el.child(
                    div()
                        .absolute()
                        .left_0()
                        .top(px(6.))
                        .bottom(px(6.))
                        .w(px(2.))
                        .bg(theme.primary),
                )
            })
            .on_click(cx.listener(move |view, _, window, cx| {
                view.pane = Pane::Runs;
                view.focus.focus(window, cx);
                view.select_run(id, true, cx);
            }))
            .child(state_icon(state, cx))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(
                        h_flex()
                            .gap_2()
                            .text_sm()
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .when(state == State::Failure, |el| el.text_color(theme.danger))
                                    .child(run.commit_subject().to_string()),
                            )
                            .child(div().flex_1())
                            .child(div().flex_none().text_xs().text_color(muted).child(age)),
                    )
                    .child(self.render_run_meta(run, is_head, duration, cx)),
            )
            .into_any_element()
    }
}

impl CiView {
    /// Second line of a run row: `workflow #n · event · [branch ·] sha · actor` and the duration
    /// on the right. Names shrink (ellipsis) so the duration never leaves the row; one font and
    /// one line height keep every part on the same baseline.
    fn render_run_meta(&self, run: &Run, is_head: bool, duration: String, cx: &App) -> Div {
        let theme = cx.theme();
        let fixed = |text: String| div().flex_none().child(text);
        let shrinking = |text: String| div().min_w_0().flex_shrink(1.).truncate().child(text);
        let dot = || div().flex_none().opacity(0.6).child("·");
        let mut parts: Vec<Div> = vec![
            shrinking(format!("{} #{}", run.workflow_name(), run.run_number)).font_weight(FontWeight::MEDIUM),
            fixed(run.event.clone()),
        ];
        if self.scope == Scope::All {
            parts.push(shrinking(run.head_branch.clone().unwrap_or_default()));
        }
        parts.push(fixed(run.short_sha().to_string()).when(is_head, |el| el.text_color(theme.primary)));
        if let Some(actor) = &run.actor {
            parts.push(shrinking(actor.login.clone()));
        }
        let mut line = h_flex()
            .min_w_0()
            .flex_1()
            .gap_1()
            .overflow_hidden()
            .whitespace_nowrap();
        for (ix, part) in parts.into_iter().enumerate() {
            if ix > 0 {
                line = line.child(dot());
            }
            line = line.child(part);
        }
        h_flex()
            .w_full()
            .gap_2()
            .text_xs()
            .line_height(px(16.))
            .text_color(theme.muted_foreground)
            .child(line)
            .child(div().flex_none().child(duration))
    }
}

impl Render for CiView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let body = if self.setup == Setup::Ready {
            v_flex()
                .size_full()
                .children(self.error.clone().map(|error| {
                    h_flex()
                        .flex_none()
                        .gap_2()
                        .px_3()
                        .py_1p5()
                        .bg(theme.danger.opacity(0.1))
                        .text_xs()
                        .text_color(theme.danger)
                        .child(div().flex_1().child(error))
                        .child(
                            Button::new("ci-error-retry")
                                .xsmall()
                                .ghost()
                                .label("Retry")
                                .on_click(cx.listener(|view, _, _, cx| view.refresh(cx))),
                        )
                }))
                .child(
                    div().flex_1().overflow_hidden().child(
                        h_resizable("ci-split")
                            .child(
                                resizable_panel()
                                    .size(px(380.))
                                    .size_range(px(260.)..px(720.))
                                    .child(
                                        div()
                                            .size_full()
                                            .border_r_1()
                                            .border_color(theme.border)
                                            .child(self.render_runs(cx)),
                                    ),
                            )
                            .child(resizable_panel().child(self.render_detail(cx))),
                    ),
                )
                .into_any_element()
        } else {
            self.render_setup(cx)
        };
        v_flex()
            .key_context(CI_CONTEXT)
            .track_focus(&self.focus)
            .size_full()
            .bg(theme.background)
            .on_action(cx.listener(|view, _: &SelectNext, _, cx| view.move_cursor(1, cx)))
            .on_action(cx.listener(|view, _: &SelectPrev, _, cx| view.move_cursor(-1, cx)))
            .on_action(cx.listener(|view, _: &OpenSelected, window, cx| view.open_selected(window, cx)))
            .on_action(cx.listener(|view, _: &Back, window, cx| view.back(window, cx)))
            .on_action(cx.listener(|view, _: &Refresh, _, cx| view.refresh(cx)))
            .on_action(cx.listener(|view, _: &OpenOnGitHub, _, cx| view.open_on_github(cx)))
            .on_action(cx.listener(|view, _: &CopySelection, _, cx| view.copy_selection(cx)))
            .on_action(cx.listener(|view, _: &RerunFailed, window, cx| {
                view.run_action(RunAction::RerunFailed, window, cx);
            }))
            .on_action(cx.listener(|view, _: &RunWorkflow, window, cx| view.open_run_dialog(window, cx)))
            .on_action(cx.listener(|view, _: &SendToClaude, window, cx| {
                view.send_failure(None, Deliver::Claude, window, cx);
            }))
            .on_action(cx.listener(|view, _: &FindInLog, window, cx| {
                view.search.update(cx, |input, cx| input.focus(window, cx));
            }))
            .child(self.render_header(cx))
            .child(div().flex_1().overflow_hidden().child(body))
    }
}
