//! The Push dialog's file tree: changed files grouped by directory, with single-child
//! directory chains merged (`crates/app`) as in JetBrains.

use std::collections::{BTreeMap, HashSet};

use vcs::CommitFile;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeRow {
    Dir {
        /// Full path, the key for folding.
        path: String,
        label: String,
        depth: usize,
        files: usize,
        expanded: bool,
    },
    /// Index into the file list.
    File { ix: usize, depth: usize },
}

#[derive(Default)]
struct Node {
    dirs: BTreeMap<String, Node>,
    files: Vec<usize>,
}

impl Node {
    fn count(&self) -> usize {
        self.files.len() + self.dirs.values().map(Node::count).sum::<usize>()
    }
}

/// Directories first, then files by name; folded directories hide their contents.
pub fn rows(files: &[CommitFile], collapsed: &HashSet<String>) -> Vec<TreeRow> {
    let mut root = Node::default();
    for (ix, file) in files.iter().enumerate() {
        let mut node = &mut root;
        let mut parts: Vec<&str> = file.path.split('/').collect();
        parts.pop();
        for part in parts {
            node = node.dirs.entry(part.to_string()).or_default();
        }
        node.files.push(ix);
    }
    let mut rows = Vec::new();
    emit(&root, "", 0, files, collapsed, &mut rows);
    rows
}

fn emit(
    node: &Node,
    prefix: &str,
    depth: usize,
    files: &[CommitFile],
    collapsed: &HashSet<String>,
    rows: &mut Vec<TreeRow>,
) {
    for (name, mut child) in &node.dirs {
        let mut label = name.clone();
        while child.files.is_empty()
            && child.dirs.len() == 1
            && let Some((next, grandchild)) = child.dirs.iter().next()
        {
            label = format!("{label}/{next}");
            child = grandchild;
        }
        let path = format!("{prefix}{label}");
        let expanded = !collapsed.contains(&path);
        rows.push(TreeRow::Dir {
            path: path.clone(),
            label,
            depth,
            files: child.count(),
            expanded,
        });
        if expanded {
            emit(child, &format!("{path}/"), depth + 1, files, collapsed, rows);
        }
    }
    let mut own = node.files.clone();
    own.sort_by_key(|ix| files.get(*ix).map(|file| file_name(&file.path)));
    rows.extend(own.into_iter().map(|ix| TreeRow::File { ix, depth }));
}

pub fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str) -> CommitFile {
        CommitFile {
            path: path.into(),
            orig_path: None,
            status: 'M',
        }
    }

    fn render(rows: &[TreeRow], files: &[CommitFile]) -> Vec<String> {
        rows.iter()
            .map(|row| match row {
                TreeRow::Dir {
                    label, depth, files, ..
                } => format!("{}{label}/ {files}", "  ".repeat(*depth)),
                TreeRow::File { ix, depth } => {
                    format!("{}{}", "  ".repeat(*depth), file_name(&files[*ix].path))
                }
            })
            .collect()
    }

    #[test]
    fn groups_by_directory_and_merges_single_child_chains() {
        let files: Vec<CommitFile> = [
            "crates/app/src/workspace/git_panel.rs",
            "PROGRESS.md",
            "crates/app/src/editor.rs",
            "crates/app/themes/neo.json",
            "crates/app/src/workspace/editors.rs",
        ]
        .map(file)
        .to_vec();
        assert_eq!(
            render(&rows(&files, &HashSet::new()), &files),
            [
                "crates/app/ 4",
                "  src/ 3",
                "    workspace/ 2",
                "      editors.rs",
                "      git_panel.rs",
                "    editor.rs",
                "  themes/ 1",
                "    neo.json",
                "PROGRESS.md",
            ]
        );
    }

    #[test]
    fn folded_directories_hide_their_files() {
        let files: Vec<CommitFile> = ["a/x.rs", "a/b/y.rs", "z.rs"].map(file).to_vec();
        let collapsed = HashSet::from(["a/b".to_string()]);
        assert_eq!(
            render(&rows(&files, &collapsed), &files),
            ["a/ 2", "  b/ 1", "  x.rs", "z.rs"]
        );
    }
}
