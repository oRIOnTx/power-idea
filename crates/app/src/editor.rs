//! Editor tab model and JetBrains-style line operations.
//!
//! Line operations are pure functions over the buffer text so they can be
//! tested without a window; the workspace applies the result to the editor.

use std::path::{Path, PathBuf};

use gpui_kit::component::diff::DiffState;
use gpui_kit::component::input::{EditorState, TabSize};
use gpui_kit::*;

pub struct EditorTab {
    pub path: PathBuf,
    /// Path relative to the project root, for display and search.
    pub rel: String,
    pub state: Entity<EditorState>,
    pub language: String,
    /// Text as last read from / written to disk.
    pub saved_text: SharedString,
    /// Set once a language server has opened this document.
    pub lsp: Option<crate::language::TabLsp>,
    /// Live preview for Markdown and SVG.
    pub preview: Option<crate::preview::Preview>,
    /// Breakpoint marks in the gutter and the debugger's paused-line highlight.
    pub breakpoint_marks: Option<gpui_kit::component::input::GutterDecorationCollection>,
    pub execution_mark: Option<gpui_kit::component::input::RangeDecorationCollection>,
    pub _subscriptions: Vec<Subscription>,
}

impl EditorTab {
    pub fn title(&self) -> String {
        self.rel.rsplit('/').next().unwrap_or(&self.rel).to_string()
    }

    pub fn is_dirty(&self, cx: &App) -> bool {
        self.state.read(cx).value() != self.saved_text
    }
}

/// Read-only diff shown in the editor area.
pub struct DiffTab {
    /// Identity used to reuse an already open diff.
    pub key: String,
    pub title: String,
    /// Repo path of the local change shown, for "Open file" and prev/next.
    pub change_path: Option<String>,
    pub state: Entity<DiffState>,
}

/// Raster image viewer.
pub struct ImageTab {
    pub path: PathBuf,
    /// Path relative to the project root.
    pub rel: String,
    /// Shows the image at 100% instead of fitting it into the tab.
    pub actual_size: bool,
}

// A handful of tabs per window: boxing the editor variant would not save anything noticeable.
#[allow(clippy::large_enum_variant)]
pub enum CenterTab {
    Editor(EditorTab),
    Image(ImageTab),
    Diff(DiffTab),
    Settings,
    /// Merge conflicts of the repository (state lives in the workspace).
    Conflicts,
    Console {
        title: String,
        console: Entity<crate::database::QueryConsole>,
    },
}

impl CenterTab {
    pub fn editor(&self) -> Option<&EditorTab> {
        match self {
            Self::Editor(tab) => Some(tab),
            Self::Image(_) | Self::Diff(_) | Self::Settings | Self::Conflicts | Self::Console { .. } => None,
        }
    }

    /// Project-relative path of a file tab (editor or image).
    pub fn file_rel(&self) -> Option<&str> {
        match self {
            Self::Editor(tab) => Some(&tab.rel),
            Self::Image(tab) => Some(&tab.rel),
            Self::Diff(_) | Self::Settings | Self::Conflicts | Self::Console { .. } => None,
        }
    }

    pub fn title(&self) -> String {
        match self {
            Self::Editor(tab) => tab.title(),
            Self::Image(tab) => tab.rel.rsplit('/').next().unwrap_or(&tab.rel).to_string(),
            Self::Diff(tab) => tab.title.clone(),
            Self::Settings => "Settings".into(),
            Self::Conflicts => "Conflicts".into(),
            Self::Console { title, .. } => title.clone(),
        }
    }

    pub fn is_dirty(&self, cx: &App) -> bool {
        self.editor().is_some_and(|tab| tab.is_dirty(cx))
    }
}

pub fn new_editor_state(
    text: String,
    language: &str,
    indent: crate::indentation::IndentStyle,
    window: &mut Window,
    cx: &mut App,
) -> Entity<EditorState> {
    let language = language.to_string();
    cx.new(|cx| {
        EditorState::new(window, cx)
            .language(language)
            .tab_size(TabSize {
                tab_size: indent.size,
                hard_tabs: indent.hard_tabs,
            })
            .soft_wrap(false)
            .default_value(text)
    })
}

/// Language name understood by the highlighter, derived from the file name.
pub fn language_for(path: &Path) -> String {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let special = match name {
        "Gemfile" | "Rakefile" | "Guardfile" | "Capfile" | "config.ru" | "Brewfile" | "Podfile"
        | "Vagrantfile" | "Fastfile" | "Appfile" | "Dangerfile" | "Berksfile" | "Thorfile" | ".irbrc"
        | ".pryrc" => Some("ruby"),
        "Makefile" | "makefile" | "GNUmakefile" | "Justfile" | "justfile" => Some("make"),
        // INI-style `key = value` sections read well with the TOML highlighter.
        ".gitconfig" | ".editorconfig" | ".npmrc" | ".pylintrc" | "setup.cfg" => Some("toml"),
        ".zprofile" | ".zshenv" | ".zlogin" | ".profile" => Some("bash"),
        // Real shell scripts are `sh` (they get the shell language server); the rest only
        // borrow the bash highlighter.
        ".bashrc" | ".bash_profile" | ".envrc" => Some("sh"),
        "Dockerfile" | ".zshrc" => Some("bash"),
        "CMakeLists.txt" => Some("cmake"),
        _ => None,
    };
    if let Some(lang) = special {
        return lang.into();
    }
    // dotenv files (`.env`, `.env.local`, `prod.env`) are `KEY=value` lines: shell syntax.
    if name == ".env" || name.starts_with(".env.") || name.ends_with(".env") {
        return "bash".into();
    }
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default();
    match ext {
        "rake" | "gemspec" | "ru" | "jbuilder" | "podspec" | "thor" | "rbi" | "irbrc" | "builder" => "ruby",
        "pyi" | "pyw" | "ipy" => "python",
        "json5" | "jsonl" | "ndjson" | "geojson" | "webmanifest" | "ipynb" => "json",
        "ini" | "cfg" | "conf" | "cnf" | "properties" | "prefs" | "desktop" | "service" => "toml",
        // No XML grammar: the HTML one highlights tags and attributes the same way.
        "xml" | "plist" | "xsd" | "xsl" | "xslt" | "xaml" | "csproj" | "fsproj" | "vbproj" | "vcxproj"
        | "props" | "targets" | "storyboard" | "xib" | "xhtml" | "vue" => "html",
        "markdown" | "mdown" | "mkd" => "markdown",
        "fish" | "ksh" | "command" => "bash",
        "m" | "mm" => "c",
        "hh" | "hxx" => "cpp",
        "less" => "css",
        "pgsql" | "mysql" | "ddl" => "sql",
        "mk" | "just" => "make",
        "gql" => "graphql",
        "erb" => "erb",
        "mjs" | "cjs" | "jsx" => "javascript",
        "mts" | "cts" => "typescript",
        "h" => "c",
        "hpp" | "cc" | "cxx" => "cpp",
        "zsh" => "bash",
        "bash" => "sh",
        "htm" => "html",
        "scss" => "css",
        "ex" | "exs" => "elixir",
        "psql" => "sql",
        other => other,
    }
    .to_string()
}

/// Normalizes editor language names/aliases to the ids used by language
/// server configs (`LanguageServerConfig::languages`).
pub fn canonical_language(language: &str) -> &str {
    match language {
        "rb" | "ruby" | "erb" => "ruby",
        "rs" | "rust" => "rust",
        "ts" | "mts" | "cts" | "typescript" => "typescript",
        "tsx" => "tsx",
        "js" | "jsx" | "mjs" | "cjs" | "javascript" => "javascript",
        "py" | "pyi" | "python" => "python",
        "go" => "go",
        "yml" | "yaml" => "yaml",
        "md" | "markdown" => "markdown",
        "jsonc" | "json" => "json",
        "h" | "c" => "c",
        "cc" | "cxx" | "hpp" | "cpp" => "cpp",
        "kt" | "kts" | "kotlin" => "kotlin",
        "ex" | "exs" | "elixir" => "elixir",
        // Only real shell scripts: `bash` also covers .env files and Dockerfiles, where a
        // shell language server would only report noise.
        "sh" => "sh",
        other => other,
    }
}

/// The LSP `languageId` for a canonical language, where the protocol names it differently.
pub fn lsp_language_id(language: &str) -> &str {
    match language {
        "sh" => "shellscript",
        "tsx" => "typescriptreact",
        other => other,
    }
}

pub fn line_comment_prefix(language: &str) -> Option<&'static str> {
    match language {
        "ruby" | "rb" | "python" | "py" | "bash" | "sh" | "yaml" | "yml" | "toml" | "make" | "elixir"
        | "cmake" | "graphql" => Some("#"),
        "sql" | "lua" => Some("--"),
        "markdown" | "md" | "html" | "json" | "diff" | "erb" | "ejs" => None,
        _ => Some("//"),
    }
}

/// Result of a line operation: new text and the 0-based line the cursor should land on.
#[derive(Debug, PartialEq, Eq)]
pub struct LineEdit {
    pub text: String,
    pub cursor_line: u32,
}

fn split_lines(text: &str) -> (Vec<&str>, bool) {
    let trailing_newline = text.ends_with('\n');
    let body = text.strip_suffix('\n').unwrap_or(text);
    (body.split('\n').collect(), trailing_newline)
}

fn join_lines(lines: &[impl AsRef<str>], trailing_newline: bool) -> String {
    let mut out = lines.iter().map(AsRef::as_ref).collect::<Vec<_>>().join("\n");
    if trailing_newline {
        out.push('\n');
    }
    out
}

pub fn toggle_line_comment(text: &str, line: u32, prefix: &str) -> Option<LineEdit> {
    let (lines, nl) = split_lines(text);
    let ix = line as usize;
    let current = *lines.get(ix)?;
    let indent_len = current.len() - current.trim_start().len();
    let (indent, rest) = current.split_at(indent_len);
    let toggled = if let Some(uncommented) = rest.strip_prefix(prefix) {
        format!("{indent}{}", uncommented.strip_prefix(' ').unwrap_or(uncommented))
    } else if rest.is_empty() {
        return None;
    } else {
        format!("{indent}{prefix} {rest}")
    };
    let mut lines: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    lines[ix] = toggled;
    let cursor_line = (line + 1).min(lines.len() as u32 - 1);
    Some(LineEdit {
        text: join_lines(&lines, nl),
        cursor_line,
    })
}

pub fn duplicate_line(text: &str, line: u32) -> Option<LineEdit> {
    let (mut lines, nl) = split_lines(text);
    let ix = line as usize;
    let current = *lines.get(ix)?;
    lines.insert(ix + 1, current);
    Some(LineEdit {
        text: join_lines(&lines, nl),
        cursor_line: line + 1,
    })
}

pub fn delete_line(text: &str, line: u32) -> Option<LineEdit> {
    let (mut lines, nl) = split_lines(text);
    let ix = line as usize;
    if ix >= lines.len() {
        return None;
    }
    lines.remove(ix);
    if lines.is_empty() {
        return Some(LineEdit {
            text: String::new(),
            cursor_line: 0,
        });
    }
    let cursor_line = line.min(lines.len() as u32 - 1);
    Some(LineEdit {
        text: join_lines(&lines, nl),
        cursor_line,
    })
}

pub fn move_line(text: &str, line: u32, up: bool) -> Option<LineEdit> {
    let (mut lines, nl) = split_lines(text);
    let ix = line as usize;
    let target = if up { ix.checked_sub(1)? } else { ix + 1 };
    if target >= lines.len() {
        return None;
    }
    lines.swap(ix, target);
    Some(LineEdit {
        text: join_lines(&lines, nl),
        cursor_line: target as u32,
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        canonical_language, delete_line, duplicate_line, language_for, lsp_language_id, move_line,
        toggle_line_comment,
    };

    #[test]
    fn detects_languages() {
        assert_eq!(language_for(Path::new("app/models/user.rb")), "rb");
        assert_eq!(language_for(Path::new("Gemfile")), "ruby");
        assert_eq!(language_for(Path::new("x/main.rs")), "rs");
        assert_eq!(language_for(Path::new("lib/tasks/db.rake")), "ruby");
        for env in [".env", ".env.local", ".env.example", "config/prod.env"] {
            assert_eq!(language_for(Path::new(env)), "bash", "{env}");
        }
        assert_eq!(language_for(Path::new("environment.rb")), "rb");
        assert_eq!(language_for(Path::new("Brewfile")), "ruby");
        assert_eq!(language_for(Path::new("stubs/os.pyi")), "python");
        assert_eq!(language_for(Path::new("setup.cfg")), "toml");
        assert_eq!(language_for(Path::new("php.ini")), "toml");
        assert_eq!(language_for(Path::new("Info.plist")), "html");
        assert_eq!(language_for(Path::new("events.jsonl")), "json");
        assert_eq!(language_for(Path::new("notes.txt")), "txt");
    }

    #[test]
    fn canonicalizes_languages() {
        assert_eq!(
            canonical_language(&language_for(Path::new("app/models/user.rb"))),
            "ruby"
        );
        assert_eq!(canonical_language(&language_for(Path::new("Gemfile"))), "ruby");
        assert_eq!(
            canonical_language(&language_for(Path::new("src/main.rs"))),
            "rust"
        );
        assert_eq!(canonical_language("tsx"), "tsx");
        assert_eq!(
            canonical_language(&language_for(Path::new("config/database.yml"))),
            "yaml"
        );
        assert_eq!(
            canonical_language(&language_for(Path::new("include/app.hpp"))),
            "cpp"
        );
        assert_eq!(canonical_language(&language_for(Path::new("bin/setup.sh"))), "sh");
        assert_eq!(
            canonical_language(&language_for(Path::new("scripts/env.bash"))),
            "sh"
        );
        assert_eq!(canonical_language(&language_for(Path::new(".envrc"))), "sh");
        assert_ne!(canonical_language(&language_for(Path::new("Dockerfile"))), "sh");
        assert_ne!(canonical_language(&language_for(Path::new(".env"))), "sh");
        assert_eq!(lsp_language_id("sh"), "shellscript");
        assert_eq!(lsp_language_id("ruby"), "ruby");
    }

    #[test]
    fn toggles_comments_preserving_indent() {
        let edit = toggle_line_comment("a\n  b\n", 1, "#").unwrap();
        assert_eq!(edit.text, "a\n  # b\n");
        assert_eq!(edit.cursor_line, 1);
        let back = toggle_line_comment(&edit.text, 1, "#").unwrap();
        assert_eq!(back.text, "a\n  b\n");
        assert!(toggle_line_comment("a\n\n", 1, "//").is_none());
    }

    #[test]
    fn duplicates_deletes_and_moves_lines() {
        assert_eq!(duplicate_line("a\nb", 0).unwrap().text, "a\na\nb");
        let deleted = delete_line("a\nb\nc\n", 2).unwrap();
        assert_eq!((deleted.text.as_str(), deleted.cursor_line), ("a\nb\n", 1));
        assert_eq!(move_line("a\nb\nc", 1, true).unwrap().text, "b\na\nc");
        assert_eq!(move_line("a\nb\nc", 1, false).unwrap().text, "a\nc\nb");
        assert!(move_line("a\nb", 0, true).is_none());
        assert!(move_line("a\nb", 1, false).is_none());
    }
}
