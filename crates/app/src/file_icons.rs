//! File-type icons for the project tree and tabs: a Lucide glyph plus a
//! theme color, so different kinds of files are easy to tell apart.

use gpui_kit::component::{ActiveTheme as _, Icon};
use gpui_kit::*;

/// Theme color slot for an icon; resolved against the active theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tint {
    Red,
    Orange,
    Yellow,
    Green,
    Cyan,
    Blue,
    Magenta,
    Muted,
}

impl Tint {
    pub fn color(self, cx: &App) -> Hsla {
        let theme = cx.theme();
        match self {
            Self::Red => theme.red,
            Self::Orange => theme.yellow.blend(theme.red.opacity(0.45)),
            Self::Yellow => theme.yellow,
            Self::Green => theme.green,
            Self::Cyan => theme.cyan,
            Self::Blue => theme.blue,
            Self::Magenta => theme.magenta,
            Self::Muted => theme.muted_foreground,
        }
    }
}

/// Lucide icon file name (without `.svg`) and tint for a file name.
pub fn file_icon(name: &str) -> (&'static str, Tint) {
    let lower = name.to_ascii_lowercase();
    let by_name = match lower.as_str() {
        "gemfile" | "gemfile.lock" | "rakefile" | "guardfile" | "capfile" => Some(("gem", Tint::Red)),
        "dockerfile" | "docker-compose.yml" | "docker-compose.yaml" | "compose.yml" => {
            Some(("container", Tint::Blue))
        }
        "makefile" | "justfile" | "procfile" => Some(("file-terminal", Tint::Green)),
        "package.json" | "cargo.toml" => Some(("package", Tint::Orange)),
        "license" | "license.md" | "copying" => Some(("scroll-text", Tint::Muted)),
        _ if lower.starts_with(".env") => Some(("key-round", Tint::Yellow)),
        _ if lower.starts_with(".git") => Some(("git-branch", Tint::Orange)),
        _ if lower.ends_with(".lock") || lower.ends_with("-lock.json") => Some(("file-lock", Tint::Muted)),
        _ if lower.ends_with("_spec.rb") || lower.ends_with("_test.rb") || lower.contains(".test.") => {
            Some(("flask-conical", Tint::Green))
        }
        _ => None,
    };
    if let Some(icon) = by_name {
        return icon;
    }

    let ext = lower.rsplit_once('.').map_or("", |(_, ext)| ext);
    match ext {
        "rb" | "rake" | "gemspec" | "ru" | "jbuilder" => ("gem", Tint::Red),
        "erb" | "html" | "htm" | "haml" | "slim" | "xml" | "vue" | "svelte" | "astro" => {
            ("code-xml", Tint::Orange)
        }
        "rs" => ("file-code", Tint::Orange),
        "js" | "mjs" | "cjs" | "jsx" => ("file-code", Tint::Yellow),
        "ts" | "mts" | "cts" | "tsx" => ("file-code", Tint::Blue),
        "py" | "pyi" => ("file-code", Tint::Cyan),
        "go" => ("file-code", Tint::Cyan),
        "java" | "kt" | "kts" | "scala" | "swift" | "c" | "h" | "cc" | "cpp" | "hpp" | "cs" | "php"
        | "ex" | "exs" | "lua" | "zig" => ("file-code", Tint::Magenta),
        "json" | "jsonc" => ("braces", Tint::Yellow),
        "yml" | "yaml" | "toml" | "ini" | "conf" | "cfg" | "properties" | "plist" => {
            ("file-cog", Tint::Magenta)
        }
        "md" | "mdx" | "txt" | "rst" | "adoc" => ("file-text", Tint::Blue),
        "css" | "scss" | "sass" | "less" => ("hash", Tint::Magenta),
        "sql" | "sqlite" | "db" => ("database", Tint::Cyan),
        "sh" | "bash" | "zsh" | "fish" => ("file-terminal", Tint::Green),
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "ico" | "webp" | "bmp" => ("file-image", Tint::Magenta),
        "csv" | "tsv" | "xlsx" => ("sheet", Tint::Green),
        "lock" => ("file-lock", Tint::Muted),
        _ => ("file", Tint::Muted),
    }
}

/// Tint for well-known folders; other folders use the muted color.
pub fn folder_tint(name: &str) -> Tint {
    match name {
        "app" | "src" | "lib" | "crates" | "packages" => Tint::Blue,
        "spec" | "test" | "tests" | "__tests__" => Tint::Green,
        "config" | ".github" | ".circleci" | ".power-idea" => Tint::Magenta,
        "db" | "migrations" => Tint::Cyan,
        "public" | "assets" | "static" => Tint::Yellow,
        _ => Tint::Muted,
    }
}

/// Ready-to-render icon for a file name.
pub fn icon_for_file(name: &str, cx: &App) -> Icon {
    let (glyph, tint) = file_icon(name);
    Icon::empty()
        .path(format!("icons/{glyph}.svg"))
        .text_color(tint.color(cx))
}

#[cfg(test)]
mod tests {
    use super::{Tint, file_icon, folder_tint};

    #[test]
    fn maps_common_files() {
        assert_eq!(file_icon("user.rb"), ("gem", Tint::Red));
        assert_eq!(file_icon("Gemfile"), ("gem", Tint::Red));
        assert_eq!(file_icon("user_spec.rb"), ("flask-conical", Tint::Green));
        assert_eq!(file_icon("index.html.erb"), ("code-xml", Tint::Orange));
        assert_eq!(file_icon("main.rs"), ("file-code", Tint::Orange));
        assert_eq!(file_icon("app.tsx"), ("file-code", Tint::Blue));
        assert_eq!(file_icon("config.yml"), ("file-cog", Tint::Magenta));
        assert_eq!(file_icon(".env.local"), ("key-round", Tint::Yellow));
        assert_eq!(file_icon(".gitignore"), ("git-branch", Tint::Orange));
        assert_eq!(file_icon("yarn.lock"), ("file-lock", Tint::Muted));
        assert_eq!(file_icon("README.md"), ("file-text", Tint::Blue));
        assert_eq!(file_icon("mystery"), ("file", Tint::Muted));
    }

    #[test]
    fn tints_known_folders() {
        assert_eq!(folder_tint("spec"), Tint::Green);
        assert_eq!(folder_tint("app"), Tint::Blue);
        assert_eq!(folder_tint("vendor"), Tint::Muted);
    }
}
