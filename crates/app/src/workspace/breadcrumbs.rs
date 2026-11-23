//! Status bar breadcrumbs (project › folders › file); a click reveals the crumb in the project tree.

use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex};
use gpui_kit::*;

use super::{LeftPanel, Workspace};
use crate::file_icons;

/// One crumb: its label and its path relative to the project root ("" is the root).
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Crumb {
    pub label: String,
    pub rel: String,
}

/// Every ancestor folder of `rel`, then `rel` itself, each with its root-relative path.
pub(super) fn crumbs(rel: &str) -> Vec<Crumb> {
    let mut path = String::new();
    rel.split('/')
        .filter(|part| !part.is_empty())
        .map(|part| {
            if !path.is_empty() {
                path.push('/');
            }
            path.push_str(part);
            Crumb {
                label: part.to_string(),
                rel: path.clone(),
            }
        })
        .collect()
}

impl Workspace {
    pub(super) fn render_breadcrumbs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (muted, muted_foreground) = (cx.theme().muted, cx.theme().muted_foreground);
        let file = self.active().map_or_default(|tab| tab.rel.clone());
        let crumbs = crumbs(&file);
        let last = crumbs.len();
        let root = Crumb {
            label: self.project_name(),
            rel: String::new(),
        };
        let separator = || {
            Icon::new(IconName::ChevronRight)
                .xsmall()
                .text_color(muted_foreground.opacity(0.6))
        };
        let mut row = h_flex().min_w_0().overflow_hidden().gap_0p5();
        for (ix, crumb) in std::iter::once(root).chain(crumbs).enumerate() {
            if ix > 0 {
                row = row.child(separator());
            }
            let icon = match ix {
                0 => Icon::new(IconName::Folder).text_color(muted_foreground),
                ix if ix == last => file_icons::icon_for_file(&crumb.label, cx),
                _ => Icon::new(IconName::Folder).text_color(file_icons::folder_tint(&crumb.label).color(cx)),
            };
            let rel = crumb.rel;
            row = row.child(
                h_flex()
                    .id(SharedString::from(format!("crumb-{ix}")))
                    .flex_shrink_0()
                    .gap_1()
                    .px_1()
                    .rounded_sm()
                    .cursor_pointer()
                    .hover(move |s| s.bg(muted))
                    .child(icon.xsmall())
                    .child(crumb.label)
                    .on_click(cx.listener(move |ws, _, _, cx| ws.reveal_in_tree(&rel, cx))),
            );
        }
        row
    }

    /// Shows the project tree with `rel` expanded to, selected and scrolled into view.
    pub(super) fn reveal_in_tree(&mut self, rel: &str, cx: &mut Context<Self>) {
        self.show_project_view = true;
        self.left_panel = LeftPanel::Project;
        if rel.is_empty() {
            self.select_root_row(cx);
            return;
        }
        let crumbs = crumbs(rel);
        if let Some((_, parents)) = crumbs.split_last() {
            self.expanded.extend(parents.iter().map(|c| c.rel.clone()));
        }
        self.tree_selection.select_only(rel);
        self.tree_cursor = Some(rel.to_string());
        self.root_selected = false;
        self.refresh_tree(cx);
        let id = SharedString::from(rel.to_string());
        self.tree.update(cx, |tree, _| {
            if let Some(ix) = tree.index_of(&id) {
                tree.scroll_to_item(ix, ScrollStrategy::Center);
            }
        });
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::{Crumb, crumbs};

    #[test]
    fn splits_a_path_into_crumbs() {
        let crumb = |label: &str, rel: &str| Crumb {
            label: label.into(),
            rel: rel.into(),
        };
        assert_eq!(
            crumbs("scripts/lib/sign.sh"),
            vec![
                crumb("scripts", "scripts"),
                crumb("lib", "scripts/lib"),
                crumb("sign.sh", "scripts/lib/sign.sh")
            ]
        );
        assert_eq!(crumbs("README.md"), vec![crumb("README.md", "README.md")]);
        assert!(crumbs("").is_empty());
    }
}
