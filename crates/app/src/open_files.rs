//! Opening files from outside a project (Finder "Open With", the Dock icon, `power-idea <file>`,
//! File → Open…): in the open project that contains them, otherwise in the LightEdit window.

use std::cell::RefCell;
use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt as _;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::component::TitleBar;
use gpui_kit::*;

use crate::workspace::{Workspace, Workspaces};

/// Files macOS asks us to open. AppKit delivers the ones a launch was started with before
/// gpui finishes launching, so they wait here until [`OpenRequests::attach`].
#[derive(Clone, Default)]
pub struct OpenRequests(Rc<RefCell<Inner>>);

#[derive(Default)]
struct Inner {
    pending: Vec<PathBuf>,
    app: Option<AsyncApp>,
}

impl OpenRequests {
    /// The `on_open_urls` handler.
    pub fn handle_urls(&self, urls: &[String]) {
        let paths: Vec<PathBuf> = urls.iter().filter_map(|url| file_url_to_path(url)).collect();
        let app = self.0.borrow().app.clone();
        match app {
            // Deferred: the open runs outside whatever AppKit callback delivered the URLs.
            Some(app) => app.update(|cx| cx.defer(move |cx| open_paths(paths, cx))),
            None => self.0.borrow_mut().pending.extend(paths),
        }
    }

    /// Connects to the running app; returns the files that arrived during launch.
    pub fn attach(&self, cx: &App) -> Vec<PathBuf> {
        let mut inner = self.0.borrow_mut();
        inner.app = Some(cx.to_async());
        std::mem::take(&mut inner.pending)
    }
}

/// `file:///Users/me/My%20Notes.md` → `/Users/me/My Notes.md`; other schemes are ignored.
pub fn file_url_to_path(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return None;
    }
    let bytes = percent_decode(rest)?;
    Some(PathBuf::from(OsString::from_vec(bytes)))
}

fn percent_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len());
    let mut bytes = text.bytes();
    while let Some(byte) = bytes.next() {
        if byte != b'%' {
            out.push(byte);
            continue;
        }
        let hex = [bytes.next()?, bytes.next()?];
        out.push(u8::from_str_radix(std::str::from_utf8(&hex).ok()?, 16).ok()?);
    }
    Some(out)
}

/// Folders open as projects, files as editor tabs; missing paths are skipped.
pub fn open_paths(paths: Vec<PathBuf>, cx: &mut App) {
    for path in paths {
        let Ok(path) = path.canonicalize() else {
            continue;
        };
        if path.is_dir() {
            crate::projects::open_project_now(path, cx);
        } else {
            open_file_now(path, cx);
        }
    }
}

/// Opens an absolute file path: in the innermost open project that contains it, else in the
/// LightEdit window (created on first use).
pub fn open_file_now(path: PathBuf, cx: &mut App) {
    let workspaces = Workspaces::live(cx);
    let project = workspaces
        .iter()
        .filter(|(_, ws)| !ws.read(cx).is_light() && path.starts_with(ws.read(cx).root()))
        .max_by_key(|(_, ws)| ws.read(cx).root().components().count());
    let light = || workspaces.iter().find(|(_, ws)| ws.read(cx).is_light());
    let Some((handle, workspace)) = project.or_else(light) else {
        return open_light_window(path, cx);
    };
    let workspace = workspace.clone();
    handle
        .update(cx, |_, window, cx| {
            window.activate_window();
            workspace.update(cx, |ws, cx| ws.open_path(&path, None, window, cx));
        })
        .ok();
    cx.activate(true);
}

fn open_light_window(path: PathBuf, cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(1100.), px(760.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(480.), px(320.))),
        ..TitleBar::window_options()
    };
    let opened = gpui_kit::open_window(options, cx, |window, cx| {
        window.set_window_title(&file_name(&path));
        cx.new(|cx| Workspace::new_light(path, window, cx))
    });
    if let Err(err) = opened {
        return eprintln!("failed to open window: {err:#}");
    }
    crate::welcome::close(cx);
    cx.activate(true);
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map_or_default(|n| n.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};

    use super::file_url_to_path;

    #[test]
    fn decodes_file_urls() {
        assert_eq!(
            file_url_to_path("file:///Users/me/My%20Notes.md"),
            Some(PathBuf::from("/Users/me/My Notes.md"))
        );
        assert_eq!(
            file_url_to_path("file://localhost/tmp/a.rb"),
            Some(PathBuf::from("/tmp/a.rb"))
        );
        // UTF-8 bytes, percent-encoded one by one.
        assert_eq!(
            file_url_to_path("file:///tmp/%D1%84%D0%B0%D0%B9%D0%BB.txt"),
            Some(PathBuf::from("/tmp/файл.txt"))
        );
        assert_eq!(file_url_to_path("https://example.com/a.rb"), None);
        assert_eq!(file_url_to_path("file:///tmp/broken%2"), None);
        assert_eq!(file_url_to_path("file:///tmp/bad%zz"), None);
    }

    /// `text-extensions.txt` feeds the app bundle's document types (scripts/bundle-macos.sh).
    #[test]
    fn text_extensions_are_well_formed() {
        let list = include_str!("../text-extensions.txt");
        let mut seen = HashSet::new();
        for ext in list
            .lines()
            .filter(|line| !line.trim_start().starts_with('#'))
            .flat_map(str::split_whitespace)
        {
            assert!(!ext.starts_with('.'), "{ext}: no leading dot");
            assert!(
                ext.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "+-_".contains(c)),
                "{ext}: plist-safe characters only"
            );
            assert!(seen.insert(ext), "{ext} is listed twice");
        }
        for ext in [
            "json", "yaml", "yml", "cfg", "py", "rb", "toml", "ini", "txt", "md",
        ] {
            assert!(seen.contains(ext), "{ext} missing");
        }
        // Every listed extension opens as text: none of them is an image preview.
        assert!(
            seen.iter()
                .all(|ext| !crate::preview::is_image(Path::new(&format!("a.{ext}"))))
        );
    }
}
