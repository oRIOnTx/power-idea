//! What rules are matched against: a project's root, files and languages.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// A snapshot of the open project, built from the IDE's file index.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectFacts {
    pub root: PathBuf,
    /// Files relative to the root, `/`-separated.
    pub files: Vec<String>,
    /// Languages found among the files (`rust`, `typescript`, …), see `language_of`.
    pub languages: BTreeSet<String>,
}

impl ProjectFacts {
    pub fn new(root: impl Into<PathBuf>, files: Vec<String>) -> Self {
        let languages = files
            .iter()
            .filter_map(|f| language_of(f))
            .map(str::to_string)
            .collect();
        Self {
            root: root.into(),
            files,
            languages,
        }
    }

    pub fn name(&self) -> String {
        self.root
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
    }
}

/// The language a file is written in, by name or extension; `None` for data and unknown files.
/// Names are the lowercase ids rules use in `applies_to.languages`.
pub fn language_of(path: &str) -> Option<&'static str> {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name {
        "Gemfile" | "Rakefile" => return Some("ruby"),
        "Dockerfile" => return Some("docker"),
        _ => {}
    }
    let ext = Path::new(name).extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "rs" => "rust",
        "ts" | "tsx" | "mts" | "cts" => "typescript",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "py" | "pyi" => "python",
        "rb" | "rake" | "erb" => "ruby",
        "go" => "go",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "swift" => "swift",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" => "cpp",
        "cs" => "csharp",
        "php" => "php",
        "ex" | "exs" => "elixir",
        "scala" => "scala",
        "sh" | "bash" | "zsh" => "shell",
        "sql" => "sql",
        "vue" => "vue",
        "svelte" => "svelte",
        "lua" => "lua",
        "zig" => "zig",
        "dart" => "dart",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_languages() {
        let facts = ProjectFacts::new(
            "/p/app",
            vec![
                "src/main.rs".into(),
                "web/App.TSX".into(),
                "Gemfile".into(),
                "README.md".into(),
            ],
        );
        let languages: Vec<&str> = facts.languages.iter().map(String::as_str).collect();
        assert_eq!(languages, ["ruby", "rust", "typescript"]);
        assert_eq!(facts.name(), "app");
        assert_eq!(language_of("notes.txt"), None);
        assert_eq!(language_of("Makefile"), None);
    }
}
