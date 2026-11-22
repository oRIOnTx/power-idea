//! Palette rows that point into files: Find Usages, File Structure,
//! Go to Symbol and Go to Class.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

/// A place in a file, with how to label it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocationItem {
    /// Symbol name, or the source line for usages.
    pub label: String,
    pub detail: Option<String>,
    /// Symbol kind (`class`, `method`…); empty for usages.
    pub kind: &'static str,
    /// Nesting level in File Structure.
    pub indent: usize,
    pub path: PathBuf,
    /// 0-based position.
    pub line: u32,
    pub column: u32,
}

/// Queries symbols for the typed text; runs on a background thread.
pub type SymbolSearch = Arc<dyn Fn(&str) -> Vec<LocationItem> + Send + Sync>;

/// `path:line` relative to `root` when inside it.
fn place(root: &Path, item: &LocationItem) -> String {
    let path = item.path.strip_prefix(root).unwrap_or(&item.path);
    format!("{}:{}", path.display(), item.line + 1)
}

pub(super) fn render_location(item: &LocationItem, root: &Path, show_place: bool, cx: &App) -> Div {
    let theme = cx.theme();
    h_flex()
        .w_full()
        .gap_2()
        .pl(px(14. * item.indent as f32))
        .children((!item.kind.is_empty()).then(|| {
            div()
                .flex_none()
                .w(px(72.))
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(item.kind)
        }))
        .child(
            div()
                .flex_none()
                .max_w(px(360.))
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .when(item.kind.is_empty(), |el| {
                    el.font_family(theme.mono_font_family.clone())
                })
                .child(item.label.clone()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .text_xs()
                .text_color(theme.muted_foreground)
                .children(item.detail.clone()),
        )
        .children(show_place.then(|| {
            div()
                .flex_none()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(place(root, item))
        }))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{LocationItem, place};

    #[test]
    fn shows_project_relative_place() {
        let item = LocationItem {
            label: "x".into(),
            detail: None,
            kind: "",
            indent: 0,
            path: PathBuf::from("/p/app/models/user.rb"),
            line: 9,
            column: 0,
        };
        assert_eq!(place(Path::new("/p"), &item), "app/models/user.rb:10");
        assert_eq!(place(Path::new("/other"), &item), "/p/app/models/user.rb:10");
    }
}
