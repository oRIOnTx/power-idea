//! Commit details pane of the Git Log: changed files, message, author.

use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use vcs::{ChangeKind, CommitFile};

use super::GitLogState;
use super::model::format_date;
use super::view::{ROW_HEIGHT, unix_now};
use crate::actions::{GitLogAction, GitLogOp};
use crate::workspace::Workspace;
use crate::workspace::git::kind_color;

impl Workspace {
    pub(super) fn render_commit_details(
        &self,
        log: &GitLogState,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let files: Vec<AnyElement> = log.details.as_ref().map_or_default(|details| {
            details
                .files
                .iter()
                .enumerate()
                .map(|(ix, file)| {
                    self.render_commit_file(ix, &details.hash, file, cx)
                        .into_any_element()
                })
                .collect()
        });
        let copy_hash = log.details.as_ref().map_or_default(|d| d.hash.clone());
        let on_copy = cx.listener(move |ws, _: &ClickEvent, window, cx| {
            let action = GitLogAction {
                op: GitLogOp::CopyRevision,
                target: copy_hash.clone().into(),
            };
            ws.handle_git_log_action(&action, window, cx);
        });
        let theme = cx.theme();
        let container = v_flex()
            .id("log-details")
            .size_full()
            .overflow_y_scroll()
            .border_l_1()
            .border_color(theme.border);
        let Some(details) = &log.details else {
            return container.items_center().justify_center().child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child("Select a commit"),
            );
        };

        let author = if details.email.is_empty() {
            details.author.clone()
        } else {
            format!("{} <{}>", details.author, details.email)
        };
        container
            .child(
                div()
                    .px_3()
                    .pt_2()
                    .pb_1()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.muted_foreground)
                    .child(format!(
                        "{} changed file{}",
                        details.files.len(),
                        if details.files.len() == 1 { "" } else { "s" }
                    )),
            )
            .children(files)
            .child(
                v_flex()
                    .mt_2()
                    .mx_2()
                    .p_2()
                    .gap_2()
                    .rounded(theme.radius)
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.background)
                    .child(div().text_sm().child(details.message.clone()))
                    .child(
                        h_flex()
                            .gap_2()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(
                                div()
                                    .id("log-hash")
                                    .font_family(theme.mono_font_family.clone())
                                    .cursor_pointer()
                                    .hover(|s| s.text_color(theme.primary))
                                    .child(details.hash.get(..10).unwrap_or(&details.hash).to_string())
                                    .on_click(on_copy),
                            )
                            .child({
                                // Recent commits also get "3 h ago"; older ones show just the date.
                                let relative =
                                    format_date(details.timestamp, unix_now(), &details.local_date);
                                if details.local_date.starts_with(&relative) {
                                    details.local_date.clone()
                                } else {
                                    format!("{} · {relative}", details.local_date)
                                }
                            }),
                    )
                    .child(div().text_xs().text_color(theme.muted_foreground).child(author)),
            )
    }

    fn render_commit_file(
        &self,
        ix: usize,
        hash: &str,
        file: &CommitFile,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let kind = file.kind();
        let color = kind_color(kind, cx);
        let (dir, name) = file.path.rsplit_once('/').unwrap_or(("", &file.path));
        let (hash, path) = (hash.to_string(), file.path.clone());
        h_flex()
            .id(("log-file", ix))
            .mx_1()
            .px_2()
            .h(ROW_HEIGHT)
            .gap_2()
            .rounded(theme.radius)
            .text_sm()
            .cursor_pointer()
            .hover(|s| s.bg(theme.list_hover))
            .child(
                div()
                    .flex_none()
                    .w(px(14.))
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(color)
                    .child(file.status.to_string()),
            )
            .child(
                div()
                    .flex_none()
                    .text_color(color)
                    .when(kind == ChangeKind::Deleted, |el| el.line_through())
                    .child(name.to_string()),
            )
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(match &file.orig_path {
                        Some(orig) => format!("← {orig}"),
                        None => dir.to_string(),
                    }),
            )
            .on_click(cx.listener(move |ws, _, window, cx| {
                ws.open_log_file_diff(hash.clone(), path.clone(), window, cx);
            }))
    }
}
