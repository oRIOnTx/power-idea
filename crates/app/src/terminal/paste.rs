//! What ⌘V sends to the PTY. Text goes as is; copied files and images (screenshots) go as
//! shell-escaped paths, the way terminals paste a Finder drop, so CLI agents such as Claude
//! Code attach the image.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use gpui_kit::{ClipboardEntry, ClipboardItem, Image};

pub(super) enum Clip {
    Text(String),
    Paths(Vec<PathBuf>),
    Image(Image),
}

/// Files first (Finder puts their names in the text entry too), then text, then an image.
pub(super) fn classify(item: &ClipboardItem) -> Option<Clip> {
    let entries = item.entries();
    let paths = entries.iter().find_map(|entry| match entry {
        ClipboardEntry::ExternalPaths(paths) if !paths.paths().is_empty() => Some(paths.paths().to_vec()),
        _ => None,
    });
    if let Some(paths) = paths {
        return Some(Clip::Paths(paths));
    }
    if let Some(text) = item.text() {
        return Some(Clip::Text(text));
    }
    entries.iter().find_map(|entry| match entry {
        ClipboardEntry::Image(image) => Some(Clip::Image(image.clone())),
        _ => None,
    })
}

/// Saves a pasted image to the temp directory; the name follows the content, so pasting the
/// same screenshot twice reuses the file.
pub(super) fn save_image(image: &Image) -> Result<PathBuf> {
    let dir = std::env::temp_dir().join("power-idea-paste");
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let path = dir.join(format!("image-{:016x}.{}", image.id, image.format.extension()));
    if !path.exists() {
        std::fs::write(&path, &image.bytes).with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(path)
}

/// Paths separated by spaces, each escaped for the shell; a trailing space like a Finder drop.
pub(super) fn paths_text(paths: &[PathBuf]) -> String {
    paths.iter().map(|p| shell_escape(p) + " ").collect()
}

fn shell_escape(path: &Path) -> String {
    let mut out = String::new();
    for ch in path.to_string_lossy().chars() {
        if !(ch.is_alphanumeric() || "/._-+,:@%~".contains(ch)) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Line endings become CR (what Enter sends); bracketed paste wraps the text when the app asked.
pub(super) fn pty_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    let text = text.replace("\r\n", "\r").replace('\n', "\r");
    if bracketed {
        format!("\x1b[200~{text}\x1b[201~").into_bytes()
    } else {
        text.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{paths_text, pty_bytes};

    #[test]
    fn escapes_paths_like_a_finder_drop() {
        let paths = [
            PathBuf::from("/tmp/a b/shot (1).png"),
            PathBuf::from("/tmp/x.png"),
        ];
        assert_eq!(paths_text(&paths), "/tmp/a\\ b/shot\\ \\(1\\).png /tmp/x.png ");
    }

    #[test]
    fn wraps_bracketed_paste() {
        assert_eq!(pty_bytes("a\nb", true), b"\x1b[200~a\rb\x1b[201~".to_vec());
        assert_eq!(pty_bytes("a\r\nb", false), b"a\rb".to_vec());
    }
}
