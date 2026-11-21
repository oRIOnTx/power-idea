//! File previews: which files get one (Markdown and SVG stay editable next to
//! their preview; raster images open in a viewer) and how Markdown resolves
//! local image links.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::{Image, SharedString};

/// A text file rendered next to its editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewKind {
    Markdown,
    Svg,
}

/// What an editor tab with a preview shows, like JetBrains' editor/split/preview toggle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewMode {
    Editor,
    Split,
    Preview,
}

impl PreviewMode {
    pub fn next(self) -> Self {
        match self {
            Self::Editor => Self::Split,
            Self::Split => Self::Preview,
            Self::Preview => Self::Editor,
        }
    }

    pub fn shows_editor(self) -> bool {
        self != Self::Preview
    }
}

/// Preview state of an editor tab.
pub struct Preview {
    pub kind: PreviewKind,
    pub mode: PreviewMode,
    /// SVG rendered from the text it was built from; rebuilt when the text changes.
    pub svg: Option<(SharedString, Arc<Image>)>,
}

impl Preview {
    pub fn new(kind: PreviewKind) -> Self {
        Self {
            kind,
            mode: PreviewMode::Split,
            svg: None,
        }
    }

    /// Returns the SVG image for `text`, re-encoding only after edits.
    pub fn svg_image(&mut self, text: &SharedString) -> Arc<Image> {
        match &self.svg {
            Some((cached, image)) if cached == text => image.clone(),
            _ => {
                let image = Arc::new(Image::from_bytes(
                    gpui_kit::ImageFormat::Svg,
                    text.as_bytes().to_vec(),
                ));
                self.svg = Some((text.clone(), image.clone()));
                image
            }
        }
    }
}

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map_or_default(str::to_ascii_lowercase)
}

/// Text files that get a live preview.
pub fn preview_kind(path: &Path) -> Option<PreviewKind> {
    match extension(path).as_str() {
        "md" | "markdown" | "mdown" | "mkd" => Some(PreviewKind::Markdown),
        "svg" => Some(PreviewKind::Svg),
        _ => None,
    }
}

/// Raster images that open in the image viewer instead of the editor.
pub fn is_image(path: &Path) -> bool {
    matches!(
        extension(path).as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "tif" | "tiff"
    )
}

/// Local file behind a Markdown image link, relative to the document's folder.
/// Remote (`http:`, `https:`) and inline (`data:`) images return `None`.
pub fn resolve_image_path(base_dir: &Path, uri: &str) -> Option<PathBuf> {
    let uri = uri.trim();
    if uri.is_empty() || ["http:", "https:", "data:"].iter().any(|p| uri.starts_with(p)) {
        return None;
    }
    let uri = uri.strip_prefix("file://").unwrap_or(uri);
    let path = uri.split(['?', '#']).next().unwrap_or(uri);
    let path = percent_decode(path);
    let path = Path::new(&path);
    Some(if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    })
}

/// Decodes `%XX` escapes (e.g. `%20`); malformed escapes are kept as is.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (byte, _) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{PreviewKind, PreviewMode, is_image, preview_kind, resolve_image_path};

    #[test]
    fn picks_preview_by_extension() {
        assert_eq!(preview_kind(Path::new("README.md")), Some(PreviewKind::Markdown));
        assert_eq!(
            preview_kind(Path::new("docs/Guide.MARKDOWN")),
            Some(PreviewKind::Markdown)
        );
        assert_eq!(preview_kind(Path::new("logo.svg")), Some(PreviewKind::Svg));
        assert_eq!(preview_kind(Path::new("main.rs")), None);
        assert!(is_image(Path::new("a/b/photo.JPG")));
        assert!(!is_image(Path::new("logo.svg")));
    }

    #[test]
    fn cycles_modes() {
        let mode = PreviewMode::Editor.next();
        assert_eq!(mode, PreviewMode::Split);
        assert!(mode.shows_editor());
        assert_eq!(mode.next(), PreviewMode::Preview);
        assert!(!PreviewMode::Preview.shows_editor());
        assert_eq!(PreviewMode::Preview.next(), PreviewMode::Editor);
    }

    #[test]
    fn resolves_local_images() {
        let base = Path::new("/p/docs");
        assert_eq!(
            resolve_image_path(base, "img/a%20b.png?raw=1"),
            Some(PathBuf::from("/p/docs/img/a b.png"))
        );
        assert_eq!(
            resolve_image_path(base, "../logo.svg#x"),
            Some(PathBuf::from("/p/docs/../logo.svg"))
        );
        assert_eq!(
            resolve_image_path(base, "file:///tmp/x.png"),
            Some(PathBuf::from("/tmp/x.png"))
        );
        assert_eq!(resolve_image_path(base, "https://x.dev/a.png"), None);
        assert_eq!(resolve_image_path(base, "data:image/png;base64,AA"), None);
        assert_eq!(
            resolve_image_path(base, "100%x.png"),
            Some(PathBuf::from("/p/docs/100%x.png"))
        );
    }
}
