//! Pure text helpers: redaction, backtrace trimming, fingerprints and time formatting.

use std::sync::LazyLock;

use regex::{Captures, Regex};

use crate::record::Kind;

/// Crates whose frames identify where an error comes from (the workspace's own crates).
pub const OWN_CRATES: &[&str] = &[
    "power_idea",
    "search",
    "vcs",
    "mcp",
    "lsp",
    "db",
    "run",
    "dap",
    "docker",
    "insights",
    "extensions",
    "claude_integration",
];

/// Own frames that go into the fingerprint.
const FINGERPRINT_FRAMES: usize = 5;
/// Frames kept on each side of an own frame.
const CONTEXT_FRAMES: usize = 2;
/// Frames kept when a backtrace has no own frames (e.g. a stripped release build).
const FALLBACK_FRAMES: usize = 12;
pub const MAX_BACKTRACE_LINES: usize = 60;
pub const MAX_BACKTRACE_BYTES: usize = 8 * 1024;

const REDACTED: &str = "[REDACTED]";

#[allow(clippy::expect_used)] // Literal patterns: covered by the tests.
fn regex(pattern: &str) -> Regex {
    Regex::new(pattern).expect("valid redaction pattern")
}

static URL_CREDENTIALS: LazyLock<Regex> =
    LazyLock::new(|| regex(r"(?i)\b([a-z][a-z0-9+.-]*://)[^/\s:@]+:[^/\s@]+@"));
static AUTH_SCHEME: LazyLock<Regex> =
    LazyLock::new(|| regex(r"(?i)\b(bearer|basic)\s+[A-Za-z0-9._~+/=-]{8,}"));
static KEY_VALUE: LazyLock<Regex> = LazyLock::new(|| {
    regex(
        r#"(?i)\b([a-z0-9_.-]*(?:password|passwd|secret|token|api[_-]?key|access[_-]?key|private[_-]?key))(["']?\s*[:=]\s*["']?)([^\s"',;&]{4,})"#,
    )
});
static KNOWN_TOKENS: LazyLock<Regex> = LazyLock::new(|| {
    regex(
        r"\b(?:sk-[A-Za-z0-9_-]{16,}|gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|xox[abprs]-[A-Za-z0-9-]{10,}|AKIA[0-9A-Z]{16}|glpat-[A-Za-z0-9_-]{20,}|eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,})",
    )
});
static OPAQUE: LazyLock<Regex> = LazyLock::new(|| regex(r"[A-Za-z0-9+_=-]{32,}"));
static FRAME_HASH: LazyLock<Regex> = LazyLock::new(|| regex(r"::h[0-9a-f]{16}$"));

/// Compiles the patterns up front so the panic hook does not pay for it.
pub fn warm_up() {
    for re in [
        &URL_CREDENTIALS,
        &AUTH_SCHEME,
        &KEY_VALUE,
        &KNOWN_TOKENS,
        &OPAQUE,
        &FRAME_HASH,
    ] {
        LazyLock::force(re);
    }
}

/// Replaces the home directory with `~` and anything that looks like a secret with `[REDACTED]`.
pub fn redact(text: &str, home: Option<&str>) -> String {
    let text = match home {
        Some(home) if home.len() > 1 => replace_home(text, home.trim_end_matches('/')),
        _ => text.to_string(),
    };
    let text = URL_CREDENTIALS.replace_all(&text, format!("${{1}}{REDACTED}@").as_str());
    let text = AUTH_SCHEME.replace_all(&text, format!("$1 {REDACTED}").as_str());
    let text = KEY_VALUE.replace_all(&text, format!("$1$2{REDACTED}").as_str());
    let text = KNOWN_TOKENS.replace_all(&text, REDACTED);
    OPAQUE
        .replace_all(&text, |caps: &Captures<'_>| {
            let word = caps.get(0).map_or("", |m| m.as_str());
            if looks_like_secret(word) {
                REDACTED.to_string()
            } else {
                word.to_string()
            }
        })
        .into_owned()
}

/// Mixed case plus digits: random keys do, identifiers and hex hashes don't.
fn looks_like_secret(word: &str) -> bool {
    word.chars().any(|c| c.is_ascii_uppercase())
        && word.chars().any(|c| c.is_ascii_lowercase())
        && word.chars().any(|c| c.is_ascii_digit())
}

/// `home` → `~`, only where it ends a path component (`/Users/al` must not eat `/Users/alice`).
fn replace_home(text: &str, home: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(ix) = rest.find(home) {
        let (before, after) = rest.split_at(ix);
        let tail = after.get(home.len()..).unwrap_or_default();
        out.push_str(before);
        let boundary = tail
            .chars()
            .next()
            .is_none_or(|c| !(c.is_alphanumeric() || matches!(c, '.' | '_' | '-')));
        out.push_str(if boundary { "~" } else { home });
        rest = tail;
    }
    out.push_str(rest);
    out
}

/// Cuts `text` to at most `max` bytes on a character boundary.
pub fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", text.get(..end).unwrap_or_default())
}

/// The message without what varies between occurrences: numbers, addresses and paths.
pub fn normalize_message(message: &str) -> String {
    let mut out = String::with_capacity(message.len());
    for (ix, word) in message.split_whitespace().enumerate() {
        if ix > 0 {
            out.push(' ');
        }
        if word.contains('/') || word.contains('\\') {
            out.push_str("<path>");
            continue;
        }
        let mut in_number = false;
        for c in word.chars() {
            if c.is_ascii_digit() {
                if !in_number {
                    out.push('#');
                }
                in_number = true;
            } else {
                in_number = false;
                out.push(c);
            }
        }
    }
    out
}

/// A stable 64-bit FNV-1a hash of the kind, the normalized message and the top own frames.
pub fn fingerprint(kind: Kind, message: &str, own_frames: &[String]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    };
    feed(kind.label().as_bytes());
    feed(&[0]);
    feed(normalize_message(message).as_bytes());
    for frame in own_frames.iter().take(FINGERPRINT_FRAMES) {
        feed(&[0]);
        feed(frame.as_bytes());
    }
    format!("{hash:016x}")
}

/// One backtrace frame: the `N: function` line and its `at file:line` lines.
struct Frame<'a> {
    function: &'a str,
    lines: Vec<&'a str>,
}

fn parse_frames(backtrace: &str) -> Vec<Frame<'_>> {
    let mut frames: Vec<Frame<'_>> = Vec::new();
    for line in backtrace.lines() {
        let trimmed = line.trim_start();
        let header = trimmed
            .split_once(": ")
            .filter(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
        if let Some((_, function)) = header {
            frames.push(Frame {
                function: function.trim(),
                lines: vec![line],
            });
        } else if let Some(frame) = frames.last_mut() {
            frame.lines.push(line);
        }
    }
    frames
}

fn bare_function(function: &str) -> &str {
    function.trim_start_matches(['<', '&']).trim_start_matches("dyn ")
}

fn is_noise(function: &str) -> bool {
    const NOISE: &[&str] = &[
        "std::",
        "core::",
        "alloc::",
        "backtrace::",
        "panic_unwind::",
        "test::",
        "error_journal::",
        "rust_begin_unwind",
        "rust_panic",
        "__rust",
        "_pthread",
        "__pthread",
        "thread_start",
    ];
    let bare = bare_function(function);
    NOISE.iter().any(|prefix| bare.starts_with(prefix)) || matches!(bare, "start" | "main" | "_main")
}

fn is_own(function: &str) -> bool {
    let bare = bare_function(function);
    OWN_CRATES.iter().any(|krate| {
        bare.strip_prefix(krate)
            .is_some_and(|rest| rest.starts_with("::"))
    })
}

/// Own frames, top first, without hash suffixes: the fingerprint's view of the stack.
pub fn own_frames(backtrace: &str) -> Vec<String> {
    parse_frames(backtrace)
        .into_iter()
        .filter(|f| is_own(f.function))
        .map(|f| FRAME_HASH.replace(f.function, "").into_owned())
        .collect()
}

/// Keeps own frames with a few neighbours, drops runtime noise and caps the size.
pub fn trim_backtrace(backtrace: &str) -> String {
    let frames: Vec<Frame<'_>> = parse_frames(backtrace)
        .into_iter()
        .filter(|f| !is_noise(f.function))
        .collect();
    if frames.is_empty() {
        let text = backtrace.trim();
        return if text.lines().count() <= 1 {
            text.to_string()
        } else {
            cap(text.lines())
        };
    }
    let own: Vec<usize> = frames
        .iter()
        .enumerate()
        .filter(|(_, frame)| is_own(frame.function))
        .map(|(ix, _)| ix)
        .collect();
    let keep = |ix: usize| {
        if own.is_empty() {
            ix < FALLBACK_FRAMES
        } else {
            own.iter().any(|&o| ix.abs_diff(o) <= CONTEXT_FRAMES)
        }
    };
    let mut lines: Vec<&str> = Vec::new();
    let mut last_kept: Option<usize> = None;
    for (ix, frame) in frames.iter().enumerate() {
        if !keep(ix) {
            continue;
        }
        if ix > 0 && last_kept.is_none_or(|last| ix > last + 1) {
            lines.push("      ⋮");
        }
        lines.extend(&frame.lines);
        last_kept = Some(ix);
    }
    if last_kept.is_some_and(|last| last + 1 < frames.len()) {
        lines.push("      ⋮");
    }
    cap(lines.into_iter())
}

fn cap<'a>(lines: impl Iterator<Item = &'a str>) -> String {
    let mut out = String::new();
    for (count, line) in lines.enumerate() {
        if count >= MAX_BACKTRACE_LINES || out.len() + line.len() + 1 > MAX_BACKTRACE_BYTES {
            out.push_str("      … (trimmed)\n");
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// `path:line[:column]` → (path, 1-based line, 1-based column).
pub fn parse_location(location: &str) -> Option<(String, u32, u32)> {
    let mut parts = location.rsplitn(3, ':');
    let last = parts.next()?;
    let middle = parts.next()?;
    match (parts.next(), middle.parse::<u32>(), last.parse::<u32>()) {
        (Some(path), Ok(line), Ok(column)) => Some((path.to_string(), line, column)),
        (None, _, Ok(line)) => Some((middle.to_string(), line, 1)),
        (Some(path), Err(_), Ok(line)) => Some((format!("{path}:{middle}"), line, 1)),
        _ => None,
    }
}

/// `2026-11-23 14:05 UTC` for unix seconds.
pub fn format_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        rem / 3600,
        rem % 3600 / 60
    )
}

/// "just now", "5m ago", "3h ago", "2d ago", then a date.
pub fn relative_time(now: u64, then: u64) -> String {
    let age = now.saturating_sub(then);
    match age {
        0..60 => "just now".into(),
        60..3_600 => format!("{}m ago", age / 60),
        3_600..86_400 => format!("{}h ago", age / 3_600),
        86_400..2_592_000 => format!("{}d ago", age / 86_400),
        _ => format_utc(then).chars().take(10).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_home_directory_on_component_boundaries() {
        let home = Some("/Users/alice");
        assert_eq!(redact("open /Users/alice/x.rs", home), "open ~/x.rs");
        assert_eq!(redact("/Users/alice", home), "~");
        assert_eq!(redact("/Users/alice2/x", home), "/Users/alice2/x");
    }

    #[test]
    fn redacts_secrets() {
        let cases = [
            (
                "Authorization: Bearer abcdef123456789",
                "Authorization: Bearer [REDACTED]",
            ),
            ("password=hunter22 next", "password=[REDACTED] next"),
            ("\"api_key\": \"s3cr3tvalue\"", "\"api_key\": \"[REDACTED]\""),
            ("postgres://bob:pw@db:5432/x", "postgres://[REDACTED]@db:5432/x"),
            ("ghp_abcdefghijklmnopqrstuvwxyz0123", "[REDACTED]"),
            ("sk-ant-api03-abcdefghijklmnop", "[REDACTED]"),
            ("key AbCdEfGh1234567890AbCdEfGh12345678", "key [REDACTED]"),
        ];
        for (input, expected) in cases {
            assert_eq!(redact(input, None), expected, "{input}");
        }
    }

    #[test]
    fn keeps_hashes_and_identifiers() {
        let sha = "commit 4190111aaaabbbbccccddddeeeeffff00001111";
        assert_eq!(redact(sha, None), sha);
        let ident = "very_long_function_name_without_secrets";
        assert_eq!(redact(ident, None), ident);
    }

    #[test]
    fn normalizes_numbers_and_paths() {
        assert_eq!(
            normalize_message("failed to read /tmp/a.txt at line 42 (0x1f)"),
            "failed to read <path> at line # (#x#f)"
        );
    }

    #[test]
    fn fingerprint_ignores_varying_parts() {
        let frames = vec!["power_idea::main".to_string()];
        let a = fingerprint(Kind::Error, "index 3 out of range in /a/b", &frames);
        let b = fingerprint(Kind::Error, "index 17 out of range in /c/d", &frames);
        assert_eq!(a, b);
        assert_ne!(
            a,
            fingerprint(Kind::Panic, "index 3 out of range in /a/b", &frames)
        );
        assert_ne!(a, fingerprint(Kind::Error, "index 3 out of range in /a/b", &[]));
    }

    const BACKTRACE: &str = "   0: std::backtrace::Backtrace::force_capture
             at /rustc/abc/library/std/src/backtrace.rs:312:13
   1: error_journal::hook::capture
   2: core::ops::function::Fn::call
   3: gpui::app::App::update
   4: gpui::window::Window::draw
   5: power_idea::workspace::Workspace::render::h0123456789abcdef
             at ./crates/app/src/workspace.rs:40:9
   6: gpui::view::render
   7: gpui::a
   8: gpui::b
   9: gpui::c
  10: gpui::d
  11: std::rt::lang_start
";

    #[test]
    fn trims_noise_and_keeps_context() {
        let trimmed = trim_backtrace(BACKTRACE);
        assert!(!trimmed.contains("force_capture"));
        assert!(!trimmed.contains("error_journal::"));
        assert!(!trimmed.contains("lang_start"));
        assert!(trimmed.contains("power_idea::workspace::Workspace::render"));
        assert!(trimmed.contains("crates/app/src/workspace.rs:40:9"));
        assert!(trimmed.contains("gpui::app::App::update"));
        assert!(trimmed.contains("gpui::a"));
        assert!(!trimmed.contains("gpui::b"));
        assert!(trimmed.trim_end().ends_with('⋮'));
    }

    #[test]
    fn own_frames_drop_hashes() {
        assert_eq!(
            own_frames(BACKTRACE),
            ["power_idea::workspace::Workspace::render"]
        );
    }

    #[test]
    fn caps_long_backtraces() {
        use std::fmt::Write as _;
        let mut long = String::new();
        for i in 0..500 {
            writeln!(
                long,
                "  {i}: power_idea::f{i}\n             at ./crates/app/src/x.rs:{i}:1"
            )
            .unwrap();
        }
        let trimmed = trim_backtrace(&long);
        assert!(trimmed.lines().count() <= MAX_BACKTRACE_LINES + 1);
        assert!(trimmed.len() <= MAX_BACKTRACE_BYTES + 32);
        assert!(trimmed.ends_with("(trimmed)\n"));
    }

    #[test]
    fn disabled_backtrace_is_kept_verbatim() {
        assert_eq!(trim_backtrace("disabled backtrace"), "disabled backtrace");
    }

    #[test]
    fn parses_locations() {
        assert_eq!(parse_location("src/a.rs:12:5"), Some(("src/a.rs".into(), 12, 5)));
        assert_eq!(parse_location("src/a.rs:12"), Some(("src/a.rs".into(), 12, 1)));
        assert_eq!(parse_location("nothing"), None);
    }

    #[test]
    fn formats_times() {
        assert_eq!(format_utc(0), "1970-01-01 00:00 UTC");
        assert_eq!(format_utc(1_795_435_500), "2026-11-23 12:05 UTC");
        assert_eq!(format_utc(951_782_400), "2000-02-29 00:00 UTC");
        assert_eq!(relative_time(1000, 990), "just now");
        assert_eq!(relative_time(10_000, 10_000 - 300), "5m ago");
        assert_eq!(relative_time(100_000, 100_000 - 7_200), "2h ago");
        assert_eq!(relative_time(1_000_000, 1_000_000 - 172_800), "2d ago");
    }
}
