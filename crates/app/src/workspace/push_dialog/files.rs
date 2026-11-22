//! The Push dialog's right side: the files the selection changes, as a folded tree.

use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use vcs::ChangeKind;

use super::tree::{self, TreeRow};
use super::{INDENT, PushDialog, ROW_HEIGHT};
use crate::workspace::git::kind_color;
use crate::{file_icons, style};

impl PushDialog {
    pub(super) fn render_files(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let panel = v_flex()
            .w(px(360.))
            .flex_none()
            .h_full()
            .border_l_1()
            .border_color(theme.border);
        let Some(files) = self.files() else {
            return panel.into_any_element();
        };
        let title = match files.len() {
            1 => "1 file".to_string(),
            n => format!("{n} files"),
        };
        let rows = tree::rows(files, &self.collapsed)
            .into_iter()
            .enumerate()
            .map(|(ix, row)| {
                let base = h_flex()
                    .id(("push-file", ix))
                    .h(px(ROW_HEIGHT))
                    .flex_none()
                    .mx_1()
                    .pr_2()
                    .gap_1p5()
                    .rounded_sm()
                    .text_sm()
                    .hover(|s| s.bg(theme.list_hover));
                match row {
                    TreeRow::Dir {
                        path,
                        label,
                        depth,
                        files,
                        expanded,
                    } => base
                        .pl(px(8. + INDENT * depth as f32))
                        .cursor_pointer()
                        .child(
                            Icon::new(if expanded {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .with_size(style::TREE_CHEVRON)
                            .text_color(muted),
                        )
                        .child(Icon::new(IconName::Folder).small().text_color(muted))
                        .child(label)
                        .child(div().text_xs().text_color(muted).child(files.to_string()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !this.collapsed.remove(&path) {
                                this.collapsed.insert(path.clone());
                            }
                            cx.notify();
                        }))
                        .into_any_element(),
                    TreeRow::File { ix, depth } => {
                        let file = &files[ix];
                        let name = tree::file_name(&file.path);
                        let kind = file.kind();
                        base.pl(px(8. + INDENT * depth as f32 + 14.))
                            .child(file_icons::icon_for_file(name, cx).small())
                            .child(
                                div()
                                    .flex_none()
                                    .text_color(kind_color(kind, cx))
                                    .when(kind == ChangeKind::Deleted, |el| el.line_through())
                                    .child(name.to_string()),
                            )
                            .children(file.orig_path.clone().map(|orig| {
                                div()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .whitespace_nowrap()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(format!("← {orig}"))
                            }))
                            .into_any_element()
                    }
                }
            });
        panel
            .child(
                div()
                    .px_3()
                    .py_1p5()
                    .text_xs()
                    .text_color(muted)
                    .border_b_1()
                    .border_color(theme.border)
                    .child(title),
            )
            .child(
                v_flex()
                    .id("push-files")
                    .flex_1()
                    .py_1()
                    .overflow_y_scroll()
                    .children(rows),
            )
            .into_any_element()
    }
}
