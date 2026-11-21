//! Lazily-loaded project file tree. Directories are read only when expanded.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use gpui_kit::component::tree::TreeItem;

/// Child used to make an unexpanded directory render as a folder.
const PLACEHOLDER: &str = "\u{0}placeholder";

const HIDDEN_NAMES: &[&str] = &[".git", ".DS_Store"];

/// Builds tree items for `dir` (relative path `rel`, empty for the root).
/// Item ids are paths relative to the project root.
pub fn build_items(root: &Path, rel: &str, expanded: &HashSet<String>) -> Vec<TreeItem> {
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
            if HIDDEN_NAMES.contains(&name.as_str()) {
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
                build_items(root, &id, expanded)
            } else {
                vec![TreeItem::new(format!("{id}/{PLACEHOLDER}"), "")]
            };
            TreeItem::new(id, name).expanded(is_expanded).children(children)
        })
        .collect()
}
