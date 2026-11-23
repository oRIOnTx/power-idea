//! Indentation of a file: `.editorconfig` first, then the project's tab size, then the
//! language's convention (Ruby 2 spaces, Go tabs, …), then the global tab size.

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndentStyle {
    pub size: usize,
    pub hard_tabs: bool,
}

impl IndentStyle {
    const fn spaces(size: usize) -> Self {
        Self {
            size,
            hard_tabs: false,
        }
    }
}

/// The community convention of a language (canonical name), where one is strong.
pub fn language_default(language: &str) -> Option<IndentStyle> {
    match language {
        "ruby" | "yaml" | "javascript" | "typescript" | "tsx" | "json" | "html" | "css" | "less"
        | "elixir" | "lua" | "erb" | "vue" | "dart" | "nix" => Some(IndentStyle::spaces(2)),
        "python" | "rust" | "java" | "kotlin" | "php" | "c" | "cpp" | "swift" | "zig" | "sql" => {
            Some(IndentStyle::spaces(4))
        }
        "go" | "make" => Some(IndentStyle {
            size: 4,
            hard_tabs: true,
        }),
        _ => None,
    }
}

/// Indentation for `path` in the project at `root`.
pub fn resolve(
    root: &Path,
    path: &Path,
    language: &str,
    project_tab: Option<usize>,
    global_tab: usize,
) -> IndentStyle {
    let fallback = project_tab
        .map(IndentStyle::spaces)
        .or_else(|| language_default(language))
        .unwrap_or(IndentStyle::spaces(global_tab));
    let settings = editorconfig::settings_for(root, path);
    let hard_tabs = settings.style.map_or(fallback.hard_tabs, |style| style == "tab");
    IndentStyle {
        size: settings.size.unwrap_or(fallback.size),
        hard_tabs,
    }
}

mod editorconfig {
    //! The indentation subset of EditorConfig (https://editorconfig.org): `indent_style`,
    //! `indent_size` and `tab_width` from the `.editorconfig` files above a file.

    use std::path::Path;

    /// A `[glob]` section and its `key = value` pairs.
    type Section = (String, Vec<(String, String)>);

    #[derive(Debug, Default, PartialEq, Eq)]
    pub struct Settings {
        pub style: Option<String>,
        pub size: Option<usize>,
    }

    /// Settings for `path`: files nearer to it win, and the search stops at `root = true`
    /// or at the project root.
    pub fn settings_for(root: &Path, path: &Path) -> Settings {
        let mut files = Vec::new();
        let mut dir = path.parent();
        while let Some(current) = dir {
            if let Ok(text) = std::fs::read_to_string(current.join(".editorconfig")) {
                let (is_root, sections) = parse(&text);
                files.push((current.to_path_buf(), sections));
                if is_root {
                    break;
                }
            }
            if current == root {
                break;
            }
            dir = current.parent();
        }
        let mut settings = Settings::default();
        // Farthest first, so nearer files and later sections override.
        for (dir, sections) in files.iter().rev() {
            let rel = path
                .strip_prefix(dir)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");
            apply(sections, &rel, &mut settings);
        }
        settings
    }

    /// `(root = true, [(glob, [(key, value)])])`, keys and values lowercased.
    fn parse(text: &str) -> (bool, Vec<Section>) {
        let mut is_root = false;
        let mut sections: Vec<Section> = Vec::new();
        for line in text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with(['#', ';']) {
                continue;
            }
            if let Some(glob) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                sections.push((glob.to_string(), Vec::new()));
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let (key, value) = (key.trim().to_lowercase(), value.trim().to_lowercase());
            match sections.last_mut() {
                Some((_, pairs)) => pairs.push((key, value)),
                None if key == "root" => is_root = value == "true",
                None => {}
            }
        }
        (is_root, sections)
    }

    fn apply(sections: &[Section], rel: &str, settings: &mut Settings) {
        for (glob, pairs) in sections {
            if !glob_matches(glob, rel) {
                continue;
            }
            let mut tab_width = None;
            for (key, value) in pairs {
                match key.as_str() {
                    "indent_style" => settings.style = Some(value.clone()),
                    "indent_size" if value == "tab" => {}
                    "indent_size" => settings.size = value.parse().ok().or(settings.size),
                    "tab_width" => tab_width = value.parse().ok(),
                    _ => {}
                }
            }
            // `indent_size = tab` (or none) with a tab width means indent by the tab width.
            let size_is_tab = pairs.iter().any(|(k, v)| k == "indent_size" && v == "tab");
            if let Some(width) = tab_width
                && (size_is_tab || settings.size.is_none())
            {
                settings.size = Some(width);
            }
        }
    }

    /// EditorConfig globs: `*` (no `/`), `**`, `?`, `{a,b}`, `[abc]`. A glob without `/`
    /// matches the file name in any directory.
    pub fn glob_matches(glob: &str, rel: &str) -> bool {
        let (glob, rel) = if glob.contains('/') {
            (glob.trim_start_matches('/'), rel)
        } else {
            (glob, rel.rsplit('/').next().unwrap_or(rel))
        };
        expand_braces(glob)
            .iter()
            .any(|g| matches(g.as_bytes(), rel.as_bytes()))
    }

    fn expand_braces(glob: &str) -> Vec<String> {
        let Some(open) = glob.find('{') else {
            return vec![glob.to_string()];
        };
        let Some(close) = glob[open..].find('}').map(|ix| open + ix) else {
            return vec![glob.to_string()];
        };
        let (head, body, tail) = (&glob[..open], &glob[open + 1..close], &glob[close + 1..]);
        body.split(',')
            .flat_map(|choice| expand_braces(&format!("{head}{choice}{tail}")))
            .collect()
    }

    fn matches(glob: &[u8], text: &[u8]) -> bool {
        // Suffixes of `text` starting at `0..=last`.
        let suffixes = |last: usize| (0..=last).filter_map(|ix| text.get(ix..));
        let first = text.first();
        let tail = text.get(1..).unwrap_or_default();
        match glob {
            [] => text.is_empty(),
            [b'*', b'*', rest @ ..] => suffixes(text.len()).any(|t| matches(rest, t)),
            // `*` stays within one path segment: it may consume up to the first `/`, not past it.
            [b'*', rest @ ..] => {
                let slash = text.iter().position(|c| *c == b'/').unwrap_or(text.len());
                suffixes(slash).any(|t| matches(rest, t))
            }
            [b'?', rest @ ..] => first.is_some_and(|c| *c != b'/') && matches(rest, tail),
            [b'[', rest @ ..] => {
                let Some(end) = rest.iter().position(|c| *c == b']') else {
                    return first == Some(&b'[') && matches(rest, tail);
                };
                let Some(first) = first else { return false };
                let (class, after) = rest.split_at(end);
                class.contains(first) && matches(after.get(1..).unwrap_or_default(), tail)
            }
            [c, rest @ ..] => first == Some(c) && matches(rest, tail),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::{Settings, glob_matches, parse};

        #[test]
        fn matches_editorconfig_globs() {
            assert!(glob_matches("*", "src/main.rs"));
            assert!(glob_matches("*.{js,ts}", "web/app.ts"));
            assert!(glob_matches("Makefile", "sub/Makefile"));
            assert!(glob_matches("lib/**.rb", "lib/a/b/c.rb"));
            assert!(!glob_matches("lib/*.rb", "lib/a/c.rb"));
            assert!(glob_matches("*.[ch]", "x/y.h"));
            assert!(!glob_matches("*.py", "x/y.rb"));
        }

        #[test]
        fn later_sections_override_earlier_ones() {
            let text = "root = true\n[*]\nindent_style = space\nindent_size = 4\n\n[*.rb]\nindent_size = 2\n[Makefile]\nindent_style = tab\n";
            let (is_root, sections) = parse(text);
            assert!(is_root);
            let settings = |rel| {
                let mut settings = Settings::default();
                super::apply(&sections, rel, &mut settings);
                settings
            };
            assert_eq!(settings("app/user.rb").size, Some(2));
            assert_eq!(settings("main.go").size, Some(4));
            assert_eq!(settings("Makefile").style.as_deref(), Some("tab"));
        }

        #[test]
        fn tab_width_sizes_tab_indentation() {
            let (_, sections) = parse("[*.go]\nindent_style = tab\nindent_size = tab\ntab_width = 8\n");
            let mut settings = Settings::default();
            super::apply(&sections, "main.go", &mut settings);
            assert_eq!(settings.size, Some(8));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::{IndentStyle, resolve};

    #[test]
    fn editorconfig_beats_project_setting_beats_language_beats_global() {
        let root = std::env::temp_dir().join(format!("power-idea-indent-{}", std::process::id()));
        fs::create_dir_all(root.join("app")).unwrap();
        let file = |rel: &str| root.join(rel);
        let spaces = |size| IndentStyle {
            size,
            hard_tabs: false,
        };

        assert_eq!(resolve(&root, &file("app/a.rb"), "ruby", None, 4), spaces(2));
        assert_eq!(resolve(&root, &file("notes.txt"), "txt", None, 4), spaces(4));
        assert_eq!(resolve(&root, &file("app/a.rb"), "ruby", Some(3), 4), spaces(3));
        assert!(resolve(&root, &file("main.go"), "go", None, 4).hard_tabs);

        fs::write(
            root.join(".editorconfig"),
            "root = true\n[*.rb]\nindent_size = 4\n",
        )
        .unwrap();
        assert_eq!(resolve(&root, &file("app/a.rb"), "ruby", Some(3), 2), spaces(4));
        fs::write(root.join("app/.editorconfig"), "[*]\nindent_style = tab\n").unwrap();
        let nested = resolve(&root, &file("app/a.rb"), "ruby", None, 2);
        assert_eq!(
            nested,
            IndentStyle {
                size: 4,
                hard_tabs: true
            }
        );
        assert_eq!(
            resolve(&root, Path::new("/elsewhere/x.rb"), "ruby", None, 4),
            spaces(2)
        );
        fs::remove_dir_all(root).unwrap();
    }
}
