//! RFC 3339 timestamps as GitHub sends them, and the short durations the panel shows.

use std::time::{SystemTime, UNIX_EPOCH};

/// Milliseconds since the Unix epoch.
pub type Millis = i64;

pub fn now_millis() -> Millis {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as Millis)
}

/// `2026-11-25T10:00:00Z`, `2026-11-25T10:00:00.1234567Z` or with a `±hh:mm` offset.
pub fn parse_rfc3339(s: &str) -> Option<Millis> {
    let s = s.trim();
    let (date, rest) = s.split_once(['T', 't', ' '])?;
    let mut date_parts = date.splitn(3, '-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: u32 = date_parts.next()?.parse().ok()?;
    let day: u32 = date_parts.next()?.parse().ok()?;

    let (time, offset_minutes) = split_offset(rest)?;
    let (hms, fraction) = time.split_once('.').unwrap_or((time, ""));
    let mut hms_parts = hms.splitn(3, ':');
    let hour: i64 = hms_parts.next()?.parse().ok()?;
    let minute: i64 = hms_parts.next()?.parse().ok()?;
    let second: i64 = hms_parts.next()?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let millis: i64 = fraction
        .chars()
        .chain(std::iter::repeat('0'))
        .take(3)
        .collect::<String>()
        .parse()
        .ok()?;
    let days = days_from_civil(year, month, day);
    let seconds = days * 86_400 + hour * 3600 + minute * 60 + second - offset_minutes * 60;
    Some(seconds * 1000 + millis)
}

fn split_offset(rest: &str) -> Option<(&str, i64)> {
    if let Some(time) = rest.strip_suffix(['Z', 'z']) {
        return Some((time, 0));
    }
    let pos = rest.rfind(['+', '-'])?;
    let (time, offset) = rest.split_at(pos);
    let sign = if offset.starts_with('-') { -1 } else { 1 };
    let (h, m) = offset.get(1..)?.split_once(':')?;
    let minutes = h.parse::<i64>().ok()? * 60 + m.parse::<i64>().ok()?;
    Some((time, sign * minutes))
}

/// Days since 1970-01-01 (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(month);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `42s`, `3m 05s`, `1h 02m`.
pub fn format_duration(millis: Millis) -> String {
    let total = millis.max(0) / 1000;
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}h {m:02}m")
    } else if m > 0 {
        format!("{m}m {s:02}s")
    } else {
        format!("{s}s")
    }
}

/// `just now`, `5m ago`, `3h ago`, `2d ago`.
pub fn format_ago(millis: Millis) -> String {
    let seconds = millis.max(0) / 1000;
    match seconds {
        0..10 => "just now".into(),
        10..60 => format!("{seconds}s ago"),
        60..3600 => format!("{}m ago", seconds / 60),
        3600..86_400 => format!("{}h ago", seconds / 3600),
        _ => format!("{}d ago", seconds / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_github_timestamps() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339("2026-11-25T10:00:00Z"), Some(1_795_600_800_000));
        assert_eq!(
            parse_rfc3339("2026-11-25T10:00:00.1234567Z"),
            Some(1_795_600_800_123)
        );
        assert_eq!(
            parse_rfc3339("2026-11-25T12:00:00+02:00"),
            Some(1_795_600_800_000)
        );
        assert_eq!(
            parse_rfc3339("2026-11-25T05:30:00-04:30"),
            Some(1_795_600_800_000)
        );
        assert_eq!(parse_rfc3339("2000-02-29T00:00:00Z"), Some(951_782_400_000));
        assert_eq!(parse_rfc3339("not a date"), None);
        assert_eq!(parse_rfc3339("2026-13-01T00:00:00Z"), None);
    }

    #[test]
    fn formats_durations_and_ages() {
        assert_eq!(format_duration(42_900), "42s");
        assert_eq!(format_duration(185_000), "3m 05s");
        assert_eq!(format_duration(3_720_000), "1h 02m");
        assert_eq!(format_duration(-5), "0s");
        assert_eq!(format_ago(3_000), "just now");
        assert_eq!(format_ago(42_000), "42s ago");
        assert_eq!(format_ago(300_000), "5m ago");
        assert_eq!(format_ago(3 * 3_600_000), "3h ago");
        assert_eq!(format_ago(2 * 86_400_000), "2d ago");
    }
}
