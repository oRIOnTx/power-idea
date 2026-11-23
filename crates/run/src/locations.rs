//! File locations in program output (stack traces, compiler errors, test
//! failures) for any language: `path:line[:column]` (Ruby, Rust, Go, Node,
//! Elixir, TypeScript…) and Python's `File "path", line N`. Only paths that
//! exist count, so ordinary text with colons is not mistaken for a location.

use std::ops::Range;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub path: PathBuf,
    /// 1-based.
    pub line: u32,
    /// 1-based, when given.
    pub column: Option<u32>,
    /// Byte range of the whole reference in the scanned text (for underlining).
    pub range: Range<usize>,
}

fn is_path_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | '~' | '@' | '+' | '\\')
}

fn digits(text: &str) -> usize {
    text.bytes().take_while(u8::is_ascii_digit).count()
}

/// Resolves a path as written in output: absolute, `~/…`, or relative to `cwd` / `root`.
fn resolve(raw: &str, root: &Path, cwd: &Path) -> Option<PathBuf> {
    let raw = raw.strip_prefix("file://").unwrap_or(raw);
    let candidates: Vec<PathBuf> = if let Some(rest) = raw.strip_prefix("~/") {
        std::env::var_os("HOME")
            .map(|home| Path::new(&home).join(rest))
            .into_iter()
            .collect()
    } else if Path::new(raw).is_absolute() {
        vec![PathBuf::from(raw)]
    } else {
        vec![cwd.join(raw), root.join(raw)]
    };
    candidates.into_iter().find(|p| p.is_file())
}

/// Every file location in `text` (one line of output), left to right.
pub fn find_locations(text: &str, root: &Path, cwd: &Path) -> Vec<Location> {
    let mut found = python_locations(text, root, cwd);
    let mut search_from = 0;
    while let Some(colon) = text[search_from..].find(':').map(|ix| search_from + ix) {
        search_from = colon + 1;
        let line_len = digits(&text[colon + 1..]);
        if line_len == 0 {
            continue;
        }
        let start = text[..colon]
            .char_indices()
            .rev()
            .take_while(|&(_, c)| is_path_char(c) || c == ':')
            .last()
            .map_or(colon, |(ix, _)| ix);
        // `file:///x` keeps its scheme; any other colon splits words (e.g. `at:foo`).
        let start = match text[start..colon].rfind(':') {
            Some(ix) if !text[start..].starts_with("file:") => start + ix + 1,
            _ => start,
        };
        let raw = &text[start..colon];
        if !raw.contains(['.', '/']) || found.iter().any(|l: &Location| l.range.contains(&start)) {
            continue;
        }
        let Some(path) = resolve(raw, root, cwd) else {
            continue;
        };
        let line_end = colon + 1 + line_len;
        let Ok(line) = text[colon + 1..line_end].parse() else {
            continue;
        };
        let (column, end) = match text[line_end..].strip_prefix(':').map(digits) {
            Some(n) if n > 0 => (
                text[line_end + 1..line_end + 1 + n].parse().ok(),
                line_end + 1 + n,
            ),
            _ => (None, line_end),
        };
        found.push(Location {
            path,
            line,
            column,
            range: start..end,
        });
        search_from = end;
    }
    found.sort_by_key(|l| l.range.start);
    found
}

/// Python tracebacks: `File "app/views.py", line 12, in index`.
fn python_locations(text: &str, root: &Path, cwd: &Path) -> Vec<Location> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(ix) = text[from..].find("File \"").map(|ix| from + ix) {
        from = ix + 6;
        let Some(close) = text[from..].find('"').map(|c| from + c) else {
            break;
        };
        let Some(rest) = text[close..].strip_prefix("\", line ") else {
            continue;
        };
        let n = digits(rest);
        let end = close + 8 + n;
        if let (Some(path), Ok(line)) = (resolve(&text[from..close], root, cwd), rest[..n].parse()) {
            found.push(Location {
                path,
                line,
                column: None,
                range: ix..end,
            });
        }
        from = end;
    }
    found
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::find_locations;

    fn project() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("power-idea-locations-{}", std::process::id()));
        for file in [
            "app/models/user.rb",
            "src/main.rs",
            "web/index.js",
            "pkg/x.go",
            "views.py",
        ] {
            let path = dir.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "x").unwrap();
        }
        dir
    }

    fn short(text: &str, root: &Path) -> Vec<(String, u32, Option<u32>, String)> {
        find_locations(text, root, root)
            .into_iter()
            .map(|l| {
                let path = l.path.strip_prefix(root).unwrap_or(&l.path).display().to_string();
                (path, l.line, l.column, text[l.range].to_string())
            })
            .collect()
    }

    #[test]
    fn finds_locations_in_common_output() {
        let root = project();
        let abs = root.join("web/index.js").display().to_string();
        assert_eq!(
            short("app/models/user.rb:12:in `save': boom", &root),
            [(
                "app/models/user.rb".into(),
                12,
                None,
                "app/models/user.rb:12".into()
            )]
        );
        assert_eq!(
            short("  --> src/main.rs:3:5", &root),
            [("src/main.rs".into(), 3, Some(5), "src/main.rs:3:5".into())]
        );
        assert_eq!(short(&format!("    at run ({abs}:10:2)"), &root)[0].1, 10);
        assert_eq!(short(&format!("at file://{abs}:7"), &root)[0].1, 7);
        assert_eq!(
            short("  File \"views.py\", line 12, in index", &root),
            [("views.py".into(), 12, None, "File \"views.py\", line 12".into())]
        );
        assert_eq!(short("./pkg/x.go:42 +0x1d", &root)[0].0, "pkg/x.go");
        // Not files, or not existing: ignored.
        assert_eq!(
            short("12:30:45 INFO started at http://localhost:3000", &root),
            [] as [(String, u32, Option<u32>, String); 0]
        );
        assert_eq!(
            short("missing.rb:3", &root),
            [] as [(String, u32, Option<u32>, String); 0]
        );
        assert_eq!(short("a src/main.rs:1 and app/models/user.rb:2", &root).len(), 2);
        let _ = fs::remove_dir_all(&root);
    }
}
