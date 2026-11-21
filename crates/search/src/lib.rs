//! Project-wide search primitives: file index with fuzzy matching and
//! plain-text "find in files". UI-agnostic so it can be reused by any frontend.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use ignore::{WalkBuilder, WalkState};
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

/// Files larger than this are skipped by text search.
const MAX_SEARCH_FILE_SIZE: u64 = 4 * 1024 * 1024;
/// Bytes inspected to decide whether a file is binary.
const BINARY_SNIFF_LEN: usize = 8 * 1024;

/// Walks `root` honoring git ignores, skipping `.git` and the `exclude`
/// paths (relative to `root`, e.g. `"tmp"` or `"public/packs"`).
fn walker(root: &Path, exclude: &[String]) -> WalkBuilder {
    let excluded: Vec<PathBuf> = exclude
        .iter()
        .map(|p| p.trim().trim_matches('/'))
        .filter(|p| !p.is_empty())
        .map(|p| root.join(p))
        .collect();
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .require_git(false)
        .filter_entry(move |entry| {
            entry.file_name() != ".git" && !excluded.iter().any(|ex| entry.path() == ex)
        });
    builder
}

/// Snapshot of all non-ignored files under a project root.
#[derive(Debug, Clone, Default)]
pub struct FileIndex {
    root: PathBuf,
    /// Paths relative to `root`, using `/` separators.
    files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMatch {
    pub path: String,
    pub score: u32,
}

impl FileIndex {
    pub fn build(root: impl Into<PathBuf>) -> Self {
        Self::build_excluding(root, &[])
    }

    pub fn build_excluding(root: impl Into<PathBuf>, exclude: &[String]) -> Self {
        let root = root.into();
        let files = Mutex::new(Vec::new());

        walker(&root, exclude).build_parallel().run(|| {
            let root = &root;
            let files = &files;
            Box::new(move |entry| {
                if let Ok(entry) = entry
                    && entry.file_type().is_some_and(|t| t.is_file())
                    && let Ok(rel) = entry.path().strip_prefix(root)
                {
                    let rel = rel.to_string_lossy().replace('\\', "/");
                    files.lock().unwrap().push(rel);
                }
                WalkState::Continue
            })
        });

        let mut files = files.into_inner().unwrap();
        files.sort_unstable();
        Self { root, files }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn files(&self) -> &[String] {
        &self.files
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Fuzzy-match files like JetBrains "Go to File": matches in the file name
    /// rank above matches spread across directories.
    pub fn fuzzy(&self, query: &str, limit: usize) -> Vec<FileMatch> {
        let query = query.trim();
        if query.is_empty() {
            return self
                .files
                .iter()
                .take(limit)
                .map(|path| FileMatch {
                    path: path.clone(),
                    score: 0,
                })
                .collect();
        }

        let mut config = Config::DEFAULT;
        config.set_match_paths();
        let mut matcher = Matcher::new(config);
        let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
        let mut buf = Vec::new();

        let mut matches: Vec<FileMatch> = self
            .files
            .iter()
            .filter_map(|path| {
                let path_score = pattern.score(Utf32Str::new(path, &mut buf), &mut matcher)?;
                let name = file_name(path);
                let name_bonus = pattern
                    .score(Utf32Str::new(name, &mut buf), &mut matcher)
                    .unwrap_or(0);
                Some(FileMatch {
                    path: path.clone(),
                    score: path_score + name_bonus * 2,
                })
            })
            .collect();

        matches.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| a.path.len().cmp(&b.path.len()))
        });
        matches.truncate(limit);
        matches
    }
}

/// Fuzzy-filters and ranks arbitrary items by a string key; keeps the
/// original order for an empty query.
pub fn fuzzy_filter<T>(query: &str, items: Vec<T>, key: impl Fn(&T) -> &str) -> Vec<T> {
    let query = query.trim();
    if query.is_empty() {
        return items;
    }
    let mut matcher = Matcher::new(Config::DEFAULT);
    let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
    let mut buf = Vec::new();
    let mut scored: Vec<(u32, T)> = items
        .into_iter()
        .filter_map(|item| {
            let score = pattern.score(Utf32Str::new(key(&item), &mut buf), &mut matcher)?;
            Some((score, item))
        })
        .collect();
    scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    scored.into_iter().map(|(_, item)| item).collect()
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TextSearchOptions {
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextMatch {
    /// Path relative to the search root.
    pub path: String,
    /// 1-based line number.
    pub line: usize,
    /// 1-based column (in chars).
    pub column: usize,
    pub line_text: String,
}

/// Plain-text "Find in Files" across a project, respecting `.gitignore`.
pub fn find_in_files(
    root: &Path,
    query: &str,
    options: TextSearchOptions,
    exclude: &[String],
) -> Vec<TextMatch> {
    if query.is_empty() {
        return Vec::new();
    }
    let limit = if options.limit == 0 {
        usize::MAX
    } else {
        options.limit
    };
    let needle = if options.case_sensitive {
        query.to_string()
    } else {
        query.to_lowercase()
    };
    let found = AtomicUsize::new(0);
    let results = Mutex::new(Vec::new());

    walker(root, exclude).build_parallel().run(|| {
        let (needle, found, results) = (&needle, &found, &results);
        Box::new(move |entry| {
            if found.load(Ordering::Relaxed) >= limit {
                return WalkState::Quit;
            }
            let Ok(entry) = entry else {
                return WalkState::Continue;
            };
            if !entry.file_type().is_some_and(|t| t.is_file())
                || entry.metadata().is_ok_and(|m| m.len() > MAX_SEARCH_FILE_SIZE)
            {
                return WalkState::Continue;
            }
            let Ok(bytes) = fs::read(entry.path()) else {
                return WalkState::Continue;
            };
            if bytes[..bytes.len().min(BINARY_SNIFF_LEN)].contains(&0) {
                return WalkState::Continue;
            }
            let text = String::from_utf8_lossy(&bytes);
            let rel = entry
                .path()
                .strip_prefix(root)
                .unwrap_or(entry.path())
                .to_string_lossy()
                .replace('\\', "/");

            let mut local = Vec::new();
            for (idx, line) in text.lines().enumerate() {
                if let Some(col) = find_in_line(line, needle, options) {
                    local.push(TextMatch {
                        path: rel.clone(),
                        line: idx + 1,
                        column: col + 1,
                        line_text: line.trim_end().to_string(),
                    });
                }
            }
            if !local.is_empty() {
                found.fetch_add(local.len(), Ordering::Relaxed);
                results.lock().unwrap().extend(local);
            }
            WalkState::Continue
        })
    });

    let mut results = results.into_inner().unwrap();
    results.sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));
    results.truncate(limit);
    results
}

/// Returns the char column of the first match of `needle` in `line`.
fn find_in_line(line: &str, needle: &str, options: TextSearchOptions) -> Option<usize> {
    let haystack = if options.case_sensitive {
        line.to_string()
    } else {
        line.to_lowercase()
    };
    let mut start = 0;
    while let Some(pos) = haystack[start..].find(needle) {
        let byte = start + pos;
        let end = byte + needle.len();
        let is_word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
        let bounded =
            !is_word(haystack[..byte].chars().next_back()) && !is_word(haystack[end..].chars().next());
        if !options.whole_word || bounded {
            return Some(haystack[..byte].chars().count());
        }
        start = end;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> tempdir::Dir {
        let dir = tempdir::Dir::new();
        dir.write("src/main.rs", "fn main() {\n    println!(\"Hello\");\n}\n");
        dir.write(
            "src/user_controller.rb",
            "class UserController\n  def hello_world; end\nend\n",
        );
        dir.write("README.md", "# hello\n");
        dir.write("target/debug/junk.rs", "hello");
        dir.write(".gitignore", "target/\n");
        dir.write("logo.bin", "\0\0hello");
        dir
    }

    #[test]
    fn index_respects_gitignore() {
        let dir = fixture();
        let index = FileIndex::build(dir.path());
        assert!(index.files().contains(&"src/main.rs".to_string()));
        assert!(index.files().contains(&".gitignore".to_string()));
        assert!(!index.files().iter().any(|f| f.starts_with("target/")));
    }

    #[test]
    fn excludes_configured_paths() {
        let dir = fixture();
        let index = FileIndex::build_excluding(dir.path(), &["src/".into()]);
        assert!(!index.files().iter().any(|f| f.starts_with("src/")));
        assert!(index.files().contains(&"README.md".to_string()));
        let hits = find_in_files(dir.path(), "hello", TextSearchOptions::default(), &["src".into()]);
        assert_eq!(
            hits.iter().map(|m| m.path.as_str()).collect::<Vec<_>>(),
            ["README.md"]
        );
    }

    #[test]
    fn fuzzy_prefers_file_name() {
        let dir = fixture();
        let index = FileIndex::build(dir.path());
        let hits = index.fuzzy("usco", 10);
        assert_eq!(
            hits.first().map(|m| m.path.as_str()),
            Some("src/user_controller.rb")
        );
    }

    #[test]
    fn fuzzy_filter_ranks_items() {
        let items = vec!["Save", "Search Everywhere", "Go to File"];
        assert_eq!(fuzzy_filter("gtf", items.clone(), |s| s), ["Go to File"]);
        assert_eq!(fuzzy_filter("", items.clone(), |s| s), items);
    }

    #[test]
    fn find_in_files_case_and_word() {
        let dir = fixture();
        let opts = TextSearchOptions::default();
        let hits = find_in_files(dir.path(), "hello", opts, &[]);
        let paths: Vec<_> = hits.iter().map(|m| m.path.as_str()).collect();
        assert_eq!(paths, ["README.md", "src/main.rs", "src/user_controller.rb"]);
        assert_eq!((hits[1].line, hits[1].column), (2, 15));

        let sensitive = TextSearchOptions {
            case_sensitive: true,
            ..opts
        };
        assert_eq!(find_in_files(dir.path(), "Hello", sensitive, &[]).len(), 1);

        let word = TextSearchOptions {
            whole_word: true,
            ..opts
        };
        let hits = find_in_files(dir.path(), "hello", word, &[]);
        assert!(hits.iter().all(|m| m.path != "src/user_controller.rb"));
    }

    /// Minimal temp dir helper to avoid an extra dev-dependency.
    mod tempdir {
        use std::path::{Path, PathBuf};
        use std::sync::atomic::{AtomicUsize, Ordering};

        pub struct Dir(PathBuf);

        impl Dir {
            pub fn new() -> Self {
                static N: AtomicUsize = AtomicUsize::new(0);
                let path = std::env::temp_dir().join(format!(
                    "power-idea-search-{}-{}",
                    std::process::id(),
                    N.fetch_add(1, Ordering::Relaxed)
                ));
                std::fs::create_dir_all(&path).unwrap();
                Self(path)
            }

            pub fn path(&self) -> &Path {
                &self.0
            }

            pub fn write(&self, rel: &str, content: &str) {
                let path = self.0.join(rel);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, content).unwrap();
            }
        }

        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }
}
