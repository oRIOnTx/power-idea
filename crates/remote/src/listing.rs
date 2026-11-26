//! Showing a folder: sorting, hidden files, the filter box and human-readable
//! sizes, dates and permissions.

use std::cmp::Ordering;

use crate::fs::Entry;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortBy {
    #[default]
    Name,
    Size,
    Modified,
}

/// Folders first, then by `by` (largest / newest first for size and date), ties by name.
pub fn sort_entries(entries: &mut [Entry], by: SortBy) {
    entries.sort_by(|a, b| {
        b.is_dir().cmp(&a.is_dir()).then_with(|| {
            let order = match by {
                SortBy::Name => Ordering::Equal,
                SortBy::Size => b.size.cmp(&a.size),
                SortBy::Modified => b.modified.cmp(&a.modified),
            };
            order.then_with(|| natural_name(&a.name).cmp(&natural_name(&b.name)))
        })
    });
}

fn natural_name(name: &str) -> String {
    name.to_lowercase()
}

/// Entries to show: hidden ones only when asked, and those matching `filter` (case-insensitive).
pub fn visible<'a>(entries: &'a [Entry], show_hidden: bool, filter: &str) -> Vec<&'a Entry> {
    let filter = filter.trim().to_lowercase();
    entries
        .iter()
        .filter(|e| show_hidden || !e.is_hidden())
        .filter(|e| filter.is_empty() || e.name.to_lowercase().contains(&filter))
        .collect()
}

/// `1.4 MB`, `512 B`.
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS.get(unit).copied().unwrap_or("B"))
    }
}

/// `rwxr-xr-x` for `0o755`.
pub fn format_mode(mode: u32) -> String {
    let mut text = String::with_capacity(9);
    for shift in [6, 3, 0] {
        let bits = (mode >> shift) & 0o7;
        text.push(if bits & 4 != 0 { 'r' } else { '-' });
        text.push(if bits & 2 != 0 { 'w' } else { '-' });
        text.push(if bits & 1 != 0 { 'x' } else { '-' });
    }
    text
}

/// Parses an octal mode (`755`, `0644`); only permission bits are accepted.
pub fn parse_octal_mode(text: &str) -> Option<u32> {
    let text = text.trim();
    if text.is_empty() || text.len() > 4 {
        return None;
    }
    u32::from_str_radix(text, 8).ok().filter(|m| *m <= 0o7777)
}

/// `2026-11-26 14:03 UTC` for seconds since the epoch.
pub fn format_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rest = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02} UTC",
        rest / 3600,
        (rest % 3600) / 60
    )
}

/// `5 min ago`, `3 days ago`.
pub fn format_age(secs: u64, now: u64) -> String {
    let age = now.saturating_sub(secs);
    match age {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", age / 60),
        3600..86_400 => format!("{} h ago", age / 3600),
        _ => format!("{} days ago", age / 86_400),
    }
}

/// Howard Hinnant's days-to-civil algorithm (proleptic Gregorian, UTC).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::EntryKind;

    fn entry(name: &str, kind: EntryKind, size: u64, modified: u64) -> Entry {
        Entry {
            name: name.into(),
            path: format!("/{name}"),
            kind,
            symlink: false,
            size,
            modified: Some(modified),
            permissions: None,
            owner: None,
            group: None,
        }
    }

    #[test]
    fn sorts_folders_first() {
        let mut entries = vec![
            entry("b.txt", EntryKind::File, 10, 1),
            entry("A.txt", EntryKind::File, 30, 3),
            entry("logs", EntryKind::Dir, 0, 2),
        ];
        sort_entries(&mut entries, SortBy::Name);
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["logs", "A.txt", "b.txt"]);
        sort_entries(&mut entries, SortBy::Size);
        assert_eq!(entries.get(1).map(|e| e.name.as_str()), Some("A.txt"));
        sort_entries(&mut entries, SortBy::Modified);
        assert_eq!(entries.get(2).map(|e| e.name.as_str()), Some("b.txt"));
    }

    #[test]
    fn filters_hidden_and_by_text() {
        let entries = vec![
            entry(".env", EntryKind::File, 1, 1),
            entry("app.log", EntryKind::File, 1, 1),
            entry("nginx", EntryKind::Dir, 1, 1),
        ];
        assert_eq!(visible(&entries, false, "").len(), 2);
        assert_eq!(visible(&entries, true, "").len(), 3);
        assert_eq!(visible(&entries, true, "LOG").len(), 1);
    }

    #[test]
    fn formats() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_mode(0o755), "rwxr-xr-x");
        assert_eq!(format_mode(0o640), "rw-r-----");
        assert_eq!(parse_octal_mode("0644"), Some(0o644));
        assert_eq!(parse_octal_mode("9"), None);
        assert_eq!(parse_octal_mode("77777"), None);
        assert_eq!(format_utc(0), "1970-01-01 00:00 UTC");
        assert_eq!(format_utc(1_795_000_000), "2026-11-18 11:06 UTC");
        assert_eq!(format_age(100, 400), "5 min ago");
    }
}
