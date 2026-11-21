//! Pure helpers of the Git Log: filtering, dates, ref names, branch tree rows.

use std::collections::{BTreeMap, HashSet};

use anyhow::{Result, bail};
use vcs::{Branch, LogEntry};

/// Case-insensitive match on subject, author or hash prefix; `query` is lowercase.
pub(super) fn matches_filter(entry: &LogEntry, query: &str) -> bool {
    query.is_empty()
        || entry.subject.to_lowercase().contains(query)
        || entry.author.to_lowercase().contains(query)
        || entry.hash.starts_with(query)
}

/// Commit date as JetBrains shows it: relative when recent, otherwise `YYYY-MM-DD` (UTC).
pub(super) fn format_date(timestamp: i64, now: i64) -> String {
    const MINUTE: i64 = 60;
    const HOUR: i64 = 60 * MINUTE;
    const DAY: i64 = 24 * HOUR;
    const TWO_DAYS: i64 = 2 * DAY;
    const WEEK: i64 = 7 * DAY;
    let age = now - timestamp;
    match age {
        ..MINUTE => "Moments ago".into(),
        MINUTE..HOUR => format!("{} min ago", age / MINUTE),
        HOUR..DAY => format!("{} h ago", age / HOUR),
        DAY..TWO_DAYS => "Yesterday".into(),
        TWO_DAYS..WEEK => format!("{} days ago", age / DAY),
        _ => {
            let (y, m, d) = civil_date(timestamp.div_euclid(DAY));
            format!("{y:04}-{m:02}-{d:02}")
        }
    }
}

/// Days since 1970-01-01 to a calendar date (Howard Hinnant's algorithm).
fn civil_date(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// Trims a user-entered branch or tag name and rejects obviously invalid ones;
/// git validates the rest.
pub(super) fn check_ref_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        bail!("Name is empty");
    }
    if name.starts_with('-') || name.contains("..") || name.chars().any(char::is_whitespace) {
        bail!("Not a valid git name: {name}");
    }
    Ok(name.to_string())
}

/// A row of the branches panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum BranchRow {
    Section {
        key: String,
        title: &'static str,
        expanded: bool,
    },
    Folder {
        key: String,
        name: String,
        depth: usize,
        expanded: bool,
    },
    Branch {
        /// Index into the branch list.
        ix: usize,
        label: String,
        depth: usize,
    },
}

/// Local and Remote sections with branches grouped into folders by `/` prefix.
pub(super) fn branch_rows(branches: &[Branch], collapsed: &HashSet<String>) -> Vec<BranchRow> {
    let mut rows = Vec::new();
    for (title, remote) in [("Local", false), ("Remote", true)] {
        let mut items: Vec<(&str, usize)> = branches
            .iter()
            .enumerate()
            .filter(|(_, b)| b.is_remote == remote)
            .map(|(ix, b)| (b.name.as_str(), ix))
            .collect();
        if items.is_empty() {
            continue;
        }
        items.sort_unstable();
        let key = title.to_string();
        let expanded = !collapsed.contains(&key);
        rows.push(BranchRow::Section {
            key: key.clone(),
            title,
            expanded,
        });
        if expanded {
            push_branch_level(&mut rows, &items, &key, 1, collapsed);
        }
    }
    rows
}

fn push_branch_level(
    rows: &mut Vec<BranchRow>,
    items: &[(&str, usize)],
    parent: &str,
    depth: usize,
    collapsed: &HashSet<String>,
) {
    let mut folders: BTreeMap<&str, Vec<(&str, usize)>> = BTreeMap::new();
    let mut leaves = Vec::new();
    for &(name, ix) in items {
        match name.split_once('/') {
            Some((folder, rest)) => folders.entry(folder).or_default().push((rest, ix)),
            None => leaves.push((name, ix)),
        }
    }
    for (name, children) in folders {
        let key = format!("{parent}/{name}");
        let expanded = !collapsed.contains(&key);
        rows.push(BranchRow::Folder {
            key: key.clone(),
            name: name.to_string(),
            depth,
            expanded,
        });
        if expanded {
            push_branch_level(rows, &children, &key, depth + 1, collapsed);
        }
    }
    rows.extend(leaves.into_iter().map(|(label, ix)| BranchRow::Branch {
        ix,
        label: label.to_string(),
        depth,
    }));
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use vcs::{Branch, LogEntry};

    use super::{BranchRow, branch_rows, check_ref_name, format_date, matches_filter};

    fn branch(name: &str, is_remote: bool) -> Branch {
        Branch {
            name: name.into(),
            is_remote,
            is_current: false,
            upstream: None,
            ahead: 0,
            behind: 0,
            upstream_gone: false,
        }
    }

    fn labels(rows: &[BranchRow]) -> Vec<String> {
        rows.iter()
            .map(|row| match row {
                BranchRow::Section { title, .. } => format!("# {title}"),
                BranchRow::Folder { name, depth, .. } => format!("{depth} {name}/"),
                BranchRow::Branch { label, depth, .. } => format!("{depth} {label}"),
            })
            .collect()
    }

    #[test]
    fn groups_branches_by_prefix() {
        let branches = [
            branch("main", false),
            branch("feature/b", false),
            branch("feature/a", false),
            branch("origin/main", true),
            branch("origin/fix/x", true),
        ];
        let rows = branch_rows(&branches, &HashSet::new());
        assert_eq!(
            labels(&rows),
            [
                "# Local",
                "1 feature/",
                "2 a",
                "2 b",
                "1 main",
                "# Remote",
                "1 origin/",
                "2 fix/",
                "3 x",
                "2 main",
            ]
        );
        assert!(matches!(rows[2], BranchRow::Branch { ix: 2, .. }));

        let collapsed = HashSet::from(["Local/feature".to_string(), "Remote".to_string()]);
        assert_eq!(
            labels(&branch_rows(&branches, &collapsed)),
            ["# Local", "1 feature/", "1 main", "# Remote"]
        );
    }

    #[test]
    fn formats_dates() {
        let now = 1_763_800_000; // 2025-11-22
        assert_eq!(format_date(now - 30, now), "Moments ago");
        assert_eq!(format_date(now - 5 * 60, now), "5 min ago");
        assert_eq!(format_date(now - 3 * 3600, now), "3 h ago");
        assert_eq!(format_date(now - 30 * 3600, now), "Yesterday");
        assert_eq!(format_date(now - 3 * 86_400, now), "3 days ago");
        assert_eq!(format_date(0, now), "1970-01-01");
        assert_eq!(format_date(951_782_400, now), "2000-02-29");
    }

    #[test]
    fn filters_and_checks_names() {
        let entry = LogEntry {
            hash: "abc123".into(),
            short_hash: "abc".into(),
            parents: Vec::new(),
            author: "Ada Lovelace".into(),
            email: String::new(),
            timestamp: 0,
            refs: Vec::new(),
            subject: "Fix Parser".into(),
        };
        assert!(matches_filter(&entry, ""));
        assert!(matches_filter(&entry, "parser"));
        assert!(matches_filter(&entry, "ada"));
        assert!(matches_filter(&entry, "abc1"));
        assert!(!matches_filter(&entry, "bc1"));

        assert_eq!(check_ref_name("  feature/x ").unwrap(), "feature/x");
        assert!(check_ref_name("").is_err());
        assert!(check_ref_name("a b").is_err());
        assert!(check_ref_name("-x").is_err());
        assert!(check_ref_name("a..b").is_err());
    }
}
