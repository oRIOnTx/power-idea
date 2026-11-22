//! Opening projects: one project per window, windows grouped as native macOS
//! tabs. When a project window is already open, the user picks where the new
//! project goes (or sets a default in Settings → Appearance).

use std::path::PathBuf;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Sizable as _, TitleBar, WindowExt as _, h_flex};
use gpui_kit::*;
use serde::{Deserialize, Serialize};

use crate::config::AppConfig;
use crate::workspace::{Workspace, Workspaces};

/// Native window tab group shared by all project windows.
const TABBING_ID: &str = "power-idea-projects";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenProjectMode {
    /// Ask every time.
    #[default]
    Ask,
    /// Replace the project in the current window.
    ThisWindow,
    /// Add a tab to the current window.
    NewTab,
}

/// Where a project window should appear.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Placement {
    NewWindow,
    NewTab,
    Replace(AnyWindowHandle),
}

/// Opens `path`, focusing it if it is already open; otherwise places it
/// according to `OpenProjectMode` relative to the active project window.
pub fn open_project(path: PathBuf, cx: &mut App) {
    let path = path.canonicalize().unwrap_or(path);
    let workspaces = Workspaces::live(cx);
    if let Some((handle, _)) = workspaces.iter().find(|(_, ws)| ws.read(cx).root() == &path) {
        handle.update(cx, |_, window, _| window.activate_window()).ok();
        return;
    }

    let active = cx.active_window();
    let Some(current) = workspaces
        .iter()
        .map(|(handle, _)| *handle)
        .find(|handle| Some(*handle) == active)
        .or_else(|| workspaces.first().map(|(handle, _)| *handle))
    else {
        return open_window(path, Placement::NewWindow, cx);
    };

    match cx.global::<AppConfig>().0.open_project_mode {
        OpenProjectMode::ThisWindow => open_window(path, Placement::Replace(current), cx),
        OpenProjectMode::NewTab => open_window(path, Placement::NewTab, cx),
        OpenProjectMode::Ask => ask_placement(path, current, cx),
    }
}

/// Shows the folder picker and opens the chosen project.
pub fn prompt_open_project(cx: &mut App) {
    let paths = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some("Open Project".into()),
    });
    cx.spawn(async move |cx| match paths.await {
        Ok(Ok(Some(mut paths))) if !paths.is_empty() => cx.update(|cx| open_project(paths.remove(0), cx)),
        // Cancelled with nothing open: fall back to the Welcome screen.
        _ => cx.update(|cx| {
            if cx.windows().is_empty() {
                crate::welcome::show(cx);
            }
        }),
    })
    .detach();
}

fn ask_placement(path: PathBuf, current: AnyWindowHandle, cx: &mut App) {
    let name = path
        .file_name()
        .map_or_default(|n| n.to_string_lossy().into_owned());
    let fallback = path.clone();
    let result = current.update(cx, |_, window, cx| {
        window.open_dialog(cx, move |dialog, _, _| {
            let choose = |placement: Placement| {
                let path = path.clone();
                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    window.close_dialog(cx);
                    open_window(path.clone(), placement, cx);
                }
            };
            dialog
                .w(px(440.))
                .title(format!("Open {name}"))
                .child("Open the project in this window or in a new tab? You can set a default in Settings.")
                .footer(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .justify_end()
                        .child(
                            Button::new("cancel")
                                .ghost()
                                .small()
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("this-window")
                                .small()
                                .label("This Window")
                                .on_click(choose(Placement::Replace(current))),
                        )
                        .child(
                            Button::new("new-tab")
                                .primary()
                                .small()
                                .label("New Tab")
                                .on_click(choose(Placement::NewTab)),
                        ),
                )
        });
    });
    if result.is_err() {
        // The window vanished meanwhile; open the project on its own.
        open_window(fallback, Placement::NewWindow, cx);
    }
}

fn window_options(cx: &App) -> WindowOptions {
    let bounds = Bounds::centered(None, size(px(1440.), px(900.)), cx);
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(640.), px(400.))),
        tabbing_identifier: Some(TABBING_ID.into()),
        ..TitleBar::window_options()
    }
}

fn open_window(path: PathBuf, placement: Placement, cx: &mut App) {
    // Replacing goes through the old window's unsaved-changes guard first.
    if let Placement::Replace(old) = placement {
        let workspace = Workspaces::live(cx)
            .into_iter()
            .find(|(h, _)| *h == old)
            .map(|(_, ws)| ws);
        let can_close = match workspace {
            Some(ws) => old
                .update(cx, |_, window, cx| {
                    ws.update(cx, |ws, cx| ws.can_close_window(window, cx))
                })
                .unwrap_or(true),
            None => true,
        };
        if !can_close {
            return;
        }
    }

    cx.global_mut::<AppConfig>().0.touch_recent_project(&path);
    crate::save_config(cx);

    let opened = gpui_kit::open_window(window_options(cx), cx, |window, cx| {
        window.set_window_title(&path.file_name().unwrap_or_default().to_string_lossy());
        cx.new(|cx| Workspace::new(path.clone(), window, cx))
    });
    let new_window = match opened {
        Ok((handle, _)) => handle,
        Err(err) => return eprintln!("failed to open window: {err:#}"),
    };
    crate::welcome::close(cx);

    match placement {
        Placement::NewWindow => {}
        Placement::NewTab => merge_into_tabs(new_window, cx),
        Placement::Replace(old) => {
            merge_into_tabs(new_window, cx);
            old.update(cx, |_, window, _| window.remove_window()).ok();
        }
    }
}

/// Groups all project windows as tabs of one window (native macOS tabs).
fn merge_into_tabs(window: AnyWindowHandle, cx: &mut App) {
    window
        .update(cx, |_, window, _| {
            window.merge_all_windows();
            window.activate_window();
        })
        .ok();
}
