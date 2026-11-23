//! The Errors tab: retention selector, Copy all / Clear all, and the recorded errors newest
//! first; a row expands to its context, location, version and backtrace.

use std::collections::HashSet;

use error_journal::{ErrorRecord, Kind, Retention};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Selectable as _, Sizable as _, WindowExt as _,
    h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::ErrorBadge;
use crate::style;
use crate::workspace::Workspace;

pub struct ErrorsView {
    workspace: WeakEntity<Workspace>,
    /// Ids of the expanded rows.
    expanded: HashSet<String>,
    _badge: Subscription,
}

impl ErrorsView {
    pub fn new(workspace: WeakEntity<Workspace>, cx: &mut Context<Self>) -> Self {
        Self {
            workspace,
            expanded: HashSet::new(),
            _badge: cx.observe_global::<ErrorBadge>(|_, cx| cx.notify()),
        }
    }

    fn toggle(&mut self, id: &str, cx: &mut Context<Self>) {
        if !self.expanded.remove(id) {
            self.expanded.insert(id.to_string());
        }
        cx.notify();
    }

    fn render_header(&self, retention: Retention, records: &[ErrorRecord], cx: &mut Context<Self>) -> Div {
        let theme = cx.theme();
        let note = cx.global::<ErrorBadge>().note.clone();
        let selector = h_flex()
            .gap_1()
            .children(Retention::ALL.iter().enumerate().map(|(ix, &option)| {
                Button::new(("error-retention", ix))
                    .xsmall()
                    .ghost()
                    .label(option.label())
                    .selected(option == retention)
                    .on_click(move |_, _, cx| super::set_retention(option, cx))
            }));
        let all = error_journal::records_to_markdown(records);
        h_flex()
            .flex_wrap()
            .gap_2()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(theme.border)
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("Errors"))
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("Keep errors for"),
            )
            .child(selector)
            .children(note.map(|note| div().text_xs().text_color(theme.muted_foreground).child(note)))
            .child(div().flex_1())
            .child(
                Button::new("errors-copy-all")
                    .xsmall()
                    .ghost()
                    .icon(IconName::Copy)
                    .label("Copy all")
                    .disabled(records.is_empty())
                    .on_click(move |_, window, cx| copy(&all, "All errors copied", window, cx)),
            )
            .child(
                Button::new("errors-clear-all")
                    .xsmall()
                    .ghost()
                    .label("Clear all")
                    .disabled(records.is_empty())
                    .on_click(|_, _, _| error_journal::clear()),
            )
    }

    fn render_row(&self, ix: usize, record: &ErrorRecord, now: u64, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let expanded = self.expanded.contains(&record.id);
        let (icon, color) = match record.kind {
            Kind::Panic => (style::lucide("bug"), theme.danger),
            Kind::Error => (Icon::new(IconName::TriangleAlert), theme.warning),
        };
        let id = record.id.clone();
        let summary = h_flex()
            .id(("error-row", ix))
            .gap_2()
            .px_4()
            .py_1p5()
            .cursor_pointer()
            .hover(|el| el.bg(theme.list_hover))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle(&id, cx)))
            .child(
                Icon::new(if expanded {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .small()
                .text_color(theme.muted_foreground),
            )
            .child(icon.small().text_color(color))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .when(!record.seen, |el| el.font_weight(FontWeight::SEMIBOLD))
                    .child(record.message.lines().next().unwrap_or_default().to_string()),
            )
            .when(record.count > 1, |el| {
                el.child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(format!("×{}", record.count)),
                )
            })
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(error_journal::relative_time(now, record.last_seen)),
            );
        v_flex()
            .border_b_1()
            .border_color(theme.border)
            .child(summary)
            .when(expanded, |el| el.child(self.render_details(ix, record, cx)))
            .into_any_element()
    }

    fn render_details(&self, ix: usize, record: &ErrorRecord, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme();
        let field = |label: &'static str, value: String| {
            h_flex()
                .gap_2()
                .text_xs()
                .child(
                    div()
                        .w(px(72.))
                        .flex_none()
                        .text_color(theme.muted_foreground)
                        .child(label),
                )
                .child(div().flex_1().min_w_0().child(value))
        };
        let mono = theme.mono_font_family.clone();
        let markdown = record.to_markdown();
        let location = record.location.clone();
        let workspace = self.workspace.clone();
        v_flex()
            .gap_1()
            .pl(px(52.))
            .pr_4()
            .pb_3()
            .when(record.message.contains('\n'), |el| {
                el.child(
                    div()
                        .text_xs()
                        .font_family(mono.clone())
                        .children(record.message.lines().map(|line| div().child(line.to_string()))),
                )
            })
            .children(record.context.clone().map(|c| field("Context", c)))
            .children(record.location.clone().map(|l| field("Location", l)))
            .children(record.thread.clone().map(|t| field("Thread", t)))
            .child(field(
                "Version",
                format!("{} (build {}) · {}", record.app_version, record.build, record.os),
            ))
            .child(field(
                "Seen",
                format!(
                    "{}× · first {} · last {}",
                    record.count,
                    error_journal::format_utc(record.first_seen),
                    error_journal::format_utc(record.last_seen)
                ),
            ))
            .when(!record.backtrace.is_empty(), |el| {
                el.child(
                    div()
                        .id(("error-backtrace", ix))
                        .mt_1()
                        .p_2()
                        .max_h(px(320.))
                        .overflow_scroll()
                        .rounded(theme.radius)
                        .bg(theme.muted)
                        .text_xs()
                        .font_family(mono)
                        .children(
                            record
                                .backtrace
                                .lines()
                                .map(|line| div().whitespace_nowrap().child(line.to_string())),
                        ),
                )
            })
            .child(
                h_flex()
                    .gap_2()
                    .mt_1()
                    .child(
                        Button::new(("error-copy", ix))
                            .xsmall()
                            .ghost()
                            .icon(IconName::Copy)
                            .label("Copy")
                            .on_click(move |_, window, cx| copy(&markdown, "Error copied", window, cx)),
                    )
                    .when_some(location, |el, location| {
                        el.child(
                            Button::new(("error-open-location", ix))
                                .xsmall()
                                .ghost()
                                .icon(IconName::FolderOpen)
                                .label("Open file location")
                                .on_click(move |_, window, cx| {
                                    if let Some(ws) = workspace.upgrade() {
                                        ws.update(cx, |ws, cx| ws.open_error_location(&location, window, cx));
                                    }
                                }),
                        )
                    }),
            )
    }
}

fn copy(text: &str, title: &'static str, window: &mut Window, cx: &mut App) {
    cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
    window.push_notification(
        Notification::success("Markdown for a GitHub issue").title(title),
        cx,
    );
}

impl Render for ErrorsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (records, retention, now) =
            error_journal::snapshot().map_or_else(Default::default, |s| (s.records, s.retention, s.now));
        let header = self.render_header(retention, &records, cx);
        let theme = cx.theme();
        let body = if records.is_empty() {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.muted_foreground)
                .child("No errors 🎉")
                .into_any_element()
        } else {
            let rows: Vec<AnyElement> = records
                .iter()
                .enumerate()
                .map(|(ix, record)| self.render_row(ix, record, now, cx))
                .collect();
            v_flex()
                .id("errors-list")
                .flex_1()
                .overflow_y_scroll()
                .children(rows)
                .into_any_element()
        };
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .child(header)
            .child(body)
    }
}
