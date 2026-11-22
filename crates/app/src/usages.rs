//! Find Usages results: usages grouped by file with the matched part of each
//! line, and the flat row list the tool window renders.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

/// One usage as reported by the language server (0-based, UTF-16 columns).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Usage {
    pub path: PathBuf,
    pub line: u32,
    pub start: u32,
    /// End column on the same line (`start` when the range spans lines).
    pub end: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageLine {
    pub line: u32,
    pub column: u32,
    /// Source line split around the match, leading indentation removed.
    pub before: String,
    pub matched: String,
    pub after: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageGroup {
    pub path: PathBuf,
    /// Shown label: project-relative path, or the full path outside the project.
    pub label: String,
    pub usages: Vec<UsageLine>,
}

/// A visible row of the tool window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageRow {
    File(usize),
    Usage(usize, usize),
}

/// Byte offset of a UTF-16 column in `line` (clamped to the line end).
fn byte_offset(line: &str, column: u32) -> usize {
    let mut units = 0;
    for (ix, ch) in line.char_indices() {
        if units >= column as usize {
            return ix;
        }
        units += ch.len_utf16();
    }
    line.len()
}

/// Splits `line` around the UTF-16 range `start..end`, trimming indentation.
pub fn split_line(line: &str, start: u32, end: u32) -> (String, String, String) {
    let indent = line.len() - line.trim_start().len();
    let start = byte_offset(line, start).max(indent);
    let end = byte_offset(line, end).max(start);
    (
        line[indent..start].to_string(),
        line[start..end].to_string(),
        line[end..].trim_end().to_string(),
    )
}

/// Groups usages by file: project files first, then by path; usages by position.
/// `read_lines` returns a file's lines (an empty list when it cannot be read).
pub fn group_usages(
    usages: Vec<Usage>,
    root: &Path,
    mut read_lines: impl FnMut(&Path) -> Vec<String>,
) -> Vec<UsageGroup> {
    let mut by_file: BTreeMap<(bool, PathBuf), Vec<Usage>> = BTreeMap::new();
    for usage in usages {
        let outside = !usage.path.starts_with(root);
        by_file
            .entry((outside, usage.path.clone()))
            .or_default()
            .push(usage);
    }
    by_file
        .into_iter()
        .map(|((_, path), mut usages)| {
            usages.sort_by_key(|u| (u.line, u.start));
            usages.dedup_by_key(|u| (u.line, u.start));
            let lines = read_lines(&path);
            let usages = usages
                .into_iter()
                .map(|u| {
                    let text = lines.get(u.line as usize).map_or("", String::as_str);
                    let (before, matched, after) = split_line(text, u.start, u.end);
                    UsageLine {
                        line: u.line,
                        column: u.start,
                        before,
                        matched,
                        after,
                    }
                })
                .collect();
            let label = path.strip_prefix(root).unwrap_or(&path).display().to_string();
            UsageGroup { path, label, usages }
        })
        .collect()
}

/// Rows to draw: each file followed by its usages unless it is collapsed.
pub fn visible_rows(groups: &[UsageGroup], collapsed: &HashSet<PathBuf>) -> Vec<UsageRow> {
    let mut rows = Vec::new();
    for (g, group) in groups.iter().enumerate() {
        rows.push(UsageRow::File(g));
        if !collapsed.contains(&group.path) {
            rows.extend((0..group.usages.len()).map(|u| UsageRow::Usage(g, u)));
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};

    use super::{Usage, UsageRow, group_usages, split_line, visible_rows};

    fn usage(path: &str, line: u32, start: u32, end: u32) -> Usage {
        Usage {
            path: PathBuf::from(path),
            line,
            start,
            end,
        }
    }

    #[test]
    fn splits_around_the_match() {
        let (before, matched, after) = split_line("    user.name = ä_name  ", 9, 13);
        assert_eq!(
            (before.as_str(), matched.as_str(), after.as_str()),
            ("user.", "name", " = ä_name")
        );
        // UTF-16 columns past a non-ASCII char.
        assert_eq!(split_line("ä foo", 2, 5).1, "foo");
        // A range inside the indentation is clamped to the code.
        assert_eq!(split_line("  x", 0, 1).0, "");
    }

    #[test]
    fn groups_by_file_project_first() {
        let usages = vec![
            usage("/gems/lib.rb", 1, 0, 1),
            usage("/p/b.rb", 3, 4, 7),
            usage("/p/a.rb", 7, 0, 3),
            usage("/p/b.rb", 0, 0, 3),
            usage("/p/b.rb", 0, 0, 3),
        ];
        let groups = group_usages(usages, Path::new("/p"), |path| {
            if path.ends_with("b.rb") {
                vec!["foo".into(), "".into(), "".into(), "  x.foo()".into()]
            } else {
                Vec::new()
            }
        });
        let labels: Vec<_> = groups
            .iter()
            .map(|g| (g.label.as_str(), g.usages.len()))
            .collect();
        assert_eq!(labels, [("a.rb", 1), ("b.rb", 2), ("/gems/lib.rb", 1)]);
        assert_eq!(groups[1].usages[1].matched, "foo");
        assert_eq!(groups[1].usages[1].before, "x.");

        let collapsed = HashSet::from([PathBuf::from("/p/b.rb")]);
        assert_eq!(
            visible_rows(&groups, &collapsed),
            [
                UsageRow::File(0),
                UsageRow::Usage(0, 0),
                UsageRow::File(1),
                UsageRow::File(2),
                UsageRow::Usage(2, 0)
            ]
        );
    }
}
