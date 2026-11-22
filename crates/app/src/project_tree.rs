//! Lazily-loaded project file tree. Directories are read only when expanded.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use gpui_kit::component::tree::TreeItem;

/// Child used to make an unexpanded directory render as a folder.
const PLACEHOLDER: &str = "\u{0}placeholder";

const HIDDEN_NAMES: &[&str] = &[".git", ".DS_Store"];

/// The IDE's own per-project folder; hidden unless the user asks to see it.
const POWER_IDEA_DIR: &str = ".power-idea";

/// What the Project tree leaves out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TreeFilter {
    pub show_power_idea_dir: bool,
}

impl TreeFilter {
    /// Whether the entry `name` directly inside `rel` is hidden.
    fn hides(self, rel: &str, name: &str) -> bool {
        HIDDEN_NAMES.contains(&name)
            || (rel.is_empty() && name == POWER_IDEA_DIR && !self.show_power_idea_dir)
    }
}

/// Builds tree items for `dir` (relative path `rel`, empty for the root).
/// Item ids are paths relative to the project root.
pub fn build_items(root: &Path, rel: &str, expanded: &HashSet<String>, filter: TreeFilter) -> Vec<TreeItem> {
    let dir = if rel.is_empty() {
        root.to_path_buf()
    } else {
        root.join(rel)
    };
    let Ok(read_dir) = fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut entries: Vec<(bool, String)> = read_dir
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if filter.hides(rel, &name) {
                return None;
            }
            let is_dir = entry.file_type().ok()?.is_dir();
            Some((is_dir, name))
        })
        .collect();
    entries.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase()))
    });

    entries
        .into_iter()
        .map(|(is_dir, name)| {
            let id = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            if !is_dir {
                return TreeItem::new(id, name);
            }
            let is_expanded = expanded.contains(&id);
            let children = if is_expanded {
                build_items(root, &id, expanded, filter)
            } else {
                vec![TreeItem::new(format!("{id}/{PLACEHOLDER}"), "")]
            };
            TreeItem::new(id, name).expanded(is_expanded).children(children)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hides_power_idea_dir_at_root_unless_shown() {
        let hidden = TreeFilter::default();
        let shown = TreeFilter {
            show_power_idea_dir: true,
        };
        assert!(hidden.hides("", ".power-idea"));
        assert!(!shown.hides("", ".power-idea"));
        assert!(!hidden.hides("docs", ".power-idea"));
        assert!(shown.hides("", ".git"));
        assert!(!hidden.hides("", "src"));
    }
}
