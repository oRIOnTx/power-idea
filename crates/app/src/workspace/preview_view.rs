//! Previews in the editor area: Markdown and SVG next to their editor
//! (editor / split / preview), and the raster image viewer.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Editor;
use gpui_kit::component::text::TextView;
use gpui_kit::component::{
    ActiveTheme as _, Selectable as _, Sizable as _, h_flex, h_resizable, resizable_panel, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::Workspace;
use crate::actions::CODE_EDITOR_CONTEXT;
use crate::editor::{CenterTab, EditorTab, ImageTab};
use crate::editor_zoom;
use crate::preview::{self, PreviewKind, PreviewMode};
use crate::style;

fn preview_message(text: &'static str, cx: &App) -> AnyElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text)
        .into_any_element()
}

impl Workspace {
    pub(super) fn open_image(
        &mut self,
        rel: &str,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let existing = self
            .tabs
            .iter()
            .position(|t| matches!(t, CenterTab::Image(image) if image.rel == rel));
        let ix = existing.unwrap_or_else(|| {
            self.tabs.push(CenterTab::Image(ImageTab {
                path,
                rel: rel.to_string(),
                actual_size: false,
            }));
            self.tabs.len() - 1
        });
        self.activate_tab(ix, window, cx);
    }

    /// Rebuilds the active SVG preview from the editor text when it changed.
    /// Called from `render`, which is the only place with `&mut self` each frame.
    pub(super) fn active_svg_image(&mut self, cx: &App) -> Option<Arc<Image>> {
        let Some(CenterTab::Editor(tab)) = self.active_tab.and_then(|ix| self.tabs.get_mut(ix)) else {
            return None;
        };
        let preview = tab.preview.as_mut().filter(|p| p.kind == PreviewKind::Svg)?;
        let text = tab.state.read(cx).value();
        Some(preview.svg_image(&text))
    }

    /// Cycles editor → split → preview for the active Markdown/SVG tab.
    pub(super) fn toggle_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mode = self
            .active()
            .and_then(|tab| tab.preview.as_ref())
            .map(|p| p.mode.next());
        if let Some(mode) = mode {
            self.set_preview_mode(mode, window, cx);
        }
    }

    fn set_preview_mode(&mut self, mode: PreviewMode, window: &mut Window, cx: &mut Context<Self>) {
        let Some(CenterTab::Editor(tab)) = self.active_tab.and_then(|ix| self.tabs.get_mut(ix)) else {
            return;
        };
        let Some(preview) = tab.preview.as_mut() else {
            return;
        };
        preview.mode = mode;
        // A hidden editor must not keep the keyboard focus.
        if mode.shows_editor() {
            self.focus_editor(window, cx);
        } else {
            self.focus_handle.focus(window, cx);
        }
        cx.notify();
    }

    /// The editor of `tab`, with its preview when the file has one.
    pub(super) fn render_editor_area(
        &self,
        tab: &EditorTab,
        svg: Option<Arc<Image>>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let editor = div()
            .key_context(CODE_EDITOR_CONTEXT)
            .size_full()
            .overflow_hidden()
            .relative()
            .child(
                Editor::new(&tab.state)
                    .bordered(false)
                    .size_full()
                    .text_size(editor_zoom::font_size(cx))
                    // Code shows `!=`, `<=`, `>=` as typed, not as `≠`, `≤`, `≥` ligatures.
                    .font_features(FontFeatures::disable_ligatures()),
            )
            .child(editor_zoom::gesture_layer())
            .into_any_element();
        let Some(preview) = &tab.preview else {
            return editor;
        };
        let content = match (preview.kind, svg) {
            (PreviewKind::Markdown, _) => self.render_markdown(tab, cx),
            (PreviewKind::Svg, Some(svg)) => render_svg(svg, cx),
            (PreviewKind::Svg, None) => preview_message("Nothing to preview", cx),
        };
        let body = match preview.mode {
            PreviewMode::Editor => editor,
            PreviewMode::Preview => content,
            PreviewMode::Split => h_resizable("preview-split")
                .child(resizable_panel().child(editor))
                .child(resizable_panel().child(content))
                .into_any_element(),
        };
        v_flex()
            .size_full()
            .child(self.render_preview_toolbar(preview.kind, preview.mode, cx))
            .child(div().flex_1().overflow_hidden().child(body))
            .into_any_element()
    }

    fn render_preview_toolbar(
        &self,
        kind: PreviewKind,
        mode: PreviewMode,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let button = |id: &'static str, icon: &str, tooltip: &'static str, target: PreviewMode| {
            style::icon_button_sized(id, style::lucide(icon), tooltip, style::BUTTON_SMALL)
                .selected(mode == target)
                .on_click(cx.listener(move |ws, _, window, cx| ws.set_preview_mode(target, window, cx)))
        };
        h_flex()
            .h(px(28.))
            .px_2()
            .gap_0p5()
            .border_b_1()
            .border_color(theme.border)
            .text_xs()
            .child(
                div()
                    .flex_1()
                    .text_color(theme.muted_foreground)
                    .child(match kind {
                        PreviewKind::Markdown => "Markdown",
                        PreviewKind::Svg => "SVG",
                    }),
            )
            .child(button(
                "preview-editor",
                "code",
                "Editor Only",
                PreviewMode::Editor,
            ))
            .child(button(
                "preview-split",
                "columns-2",
                "Editor and Preview",
                PreviewMode::Split,
            ))
            .child(button(
                "preview-only",
                "eye",
                "Preview Only",
                PreviewMode::Preview,
            ))
    }

    fn render_markdown(&self, tab: &EditorTab, cx: &mut Context<Self>) -> AnyElement {
        let text = tab.state.read(cx).value();
        let base = tab.path.parent().map_or_default(Path::to_path_buf);
        div()
            .id("markdown-preview")
            .size_full()
            .overflow_y_scroll()
            .px_6()
            .py_4()
            .bg(cx.theme().background)
            .child(
                TextView::markdown(SharedString::from(format!("md:{}", tab.rel)), text)
                    .selectable(true)
                    .image_source(
                        move |uri| match preview::resolve_image_path(&base, uri.as_ref()) {
                            Some(path) => path.into(),
                            None => uri.clone().into(),
                        },
                    ),
            )
            .into_any_element()
    }

    pub(super) fn render_image_tab(&self, tab: &ImageTab, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let actual = tab.actual_size;
        let toggle = |id: &'static str, label: &'static str, value: bool| {
            Button::new(id)
                .xsmall()
                .label(label)
                .when(actual == value, ButtonVariants::primary)
                .when(actual != value, ButtonVariants::ghost)
                .on_click(cx.listener(move |ws, _, _, cx| {
                    if let Some(CenterTab::Image(tab)) = ws.active_tab.and_then(|ix| ws.tabs.get_mut(ix)) {
                        tab.actual_size = value;
                    }
                    cx.notify();
                }))
        };
        let image = img(tab.path.clone()).with_fallback({
            let muted = theme.muted_foreground;
            move || {
                div()
                    .text_sm()
                    .text_color(muted)
                    .child("Cannot display this image")
                    .into_any_element()
            }
        });
        let body = if actual {
            div()
                .id("image-view")
                .size_full()
                .overflow_scroll()
                .p_4()
                .child(image.flex_none())
        } else {
            div()
                .id("image-view")
                .size_full()
                .p_4()
                .child(image.size_full().object_fit(ObjectFit::Contain))
        };
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .h(px(28.))
                    .px_2()
                    .gap_1()
                    .border_b_1()
                    .border_color(theme.border)
                    .text_xs()
                    .child(
                        div()
                            .flex_1()
                            .text_color(theme.muted_foreground)
                            .child(tab.rel.clone()),
                    )
                    .child(toggle("image-fit", "Fit", false))
                    .child(toggle("image-actual", "100%", true)),
            )
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .bg(theme.muted.opacity(0.3))
                    .child(body),
            )
            .into_any_element()
    }
}

fn render_svg(svg: Arc<Image>, cx: &App) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    div()
        .size_full()
        .p_4()
        .bg(cx.theme().muted.opacity(0.3))
        .child(
            img(svg)
                .size_full()
                .object_fit(ObjectFit::Contain)
                .with_fallback(move || {
                    div()
                        .text_sm()
                        .text_color(muted)
                        .child("Invalid SVG")
                        .into_any_element()
                }),
        )
        .into_any_element()
}
