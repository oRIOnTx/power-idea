//! Opening projects: one project per window, windows grouped as native macOS
//! tabs. When a project window is already open, the user picks where the new
//! project goes (or sets a default in Settings → Appearance).

use std::path::PathBuf;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, TitleBar, WindowExt as _, h_flex, v_flex};
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

/// Launch: reopens the projects that were open at quit, grouped as tabs of one window.
pub fn reopen_projects(paths: Vec<PathBuf>, cx: &mut App) {
    for (ix, path) in paths.iter().enumerate() {
        let placement = if ix == 0 {
            Placement::NewWindow
        } else {
            Placement::NewTab
        };
        open_window(path.clone(), placement, cx);
    }
    // Opening touched each one; keep the first as the most recent project.
    let config = &mut cx.global_mut::<AppConfig>().0;
    for path in paths.iter().rev() {
        config.touch_recent_project(path);
    }
    crate::save_config(cx);
}

/// Records the open project windows for the next launch (runs while quitting).
pub fn remember_open_projects(cx: &mut App) {
    let open: Vec<PathBuf> = Workspaces::live(cx)
        .iter()
        .map(|(_, ws)| ws.read(cx).root().clone())
        .collect();
    cx.global_mut::<AppConfig>().0.open_projects = Some(open);
    crate::save_config(cx);
}

/// Opens `path`, focusing it if it is already open; otherwise places it
/// according to `OpenProjectMode` relative to the active project window.
pub fn open_project(path: PathBuf, cx: &mut App) {
    // Callers are often inside a window update (palette, Welcome list); the
    // placement dialog and window juggling need that window free, so run later.
    cx.defer(move |cx| open_project_now(path, cx));
}

/// Opens right away; for callers outside any window update (app start, async tasks).
pub fn open_project_now(path: PathBuf, cx: &mut App) {
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
        Ok(Ok(Some(mut paths))) if !paths.is_empty() => cx.update(|cx| open_project_now(paths.remove(0), cx)),
        // Cancelled with nothing open: fall back to the Welcome screen.
        _ => cx.update(|cx| {
            if cx.windows().is_empty() {
                crate::welcome::show(cx);
            }
        }),
    })
    .detach();
}

/// Name of a project folder, for labels.
fn folder_name(path: &std::path::Path) -> String {
    path.file_name()
        .map_or_default(|n| n.to_string_lossy().into_owned())
}

/// JetBrains-style choice: replace the current project or add a tab next to it.
fn ask_placement(path: PathBuf, current: AnyWindowHandle, cx: &mut App) {
    let name = folder_name(&path);
    let current_name = Workspaces::live(cx)
        .into_iter()
        .find(|(handle, _)| *handle == current)
        .map_or_default(|(_, ws)| folder_name(ws.read(cx).root()));
    let remember = cx.new(|_| false);
    let fallback = path.clone();
    let result = current.update(cx, |_, window, cx| {
        window.open_dialog(cx, move |dialog, _, cx| {
            let theme = cx.theme();
            let tile =
                |id: &'static str, icon: &str, title: &'static str, detail: String, placement: Placement| {
                    let (path, remember) = (path.clone(), remember.clone());
                    let accent = theme.primary;
                    v_flex()
                        .id(id)
                        .flex_1()
                        .p_4()
                        .gap_2()
                        .rounded(theme.radius_lg)
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.background)
                        .cursor_pointer()
                        .hover(move |s| s.border_color(accent).bg(accent.opacity(0.06)))
                        .child(crate::style::lucide(icon).size_6().text_color(accent))
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
                        .child(div().text_xs().text_color(theme.muted_foreground).child(detail))
                        .on_click(move |_, window, cx| {
                            if *remember.read(cx) {
                                remember_placement(placement, cx);
                            }
                            window.close_dialog(cx);
                            open_window(path.clone(), placement, cx);
                        })
                };
            let remember_toggle = remember.clone();
            dialog
                .w(px(500.))
                .title(format!("Open “{name}”"))
                .child(
                    v_flex()
                        .gap_4()
                        .child(
                            div()
                                .text_sm()
                                .text_color(theme.muted_foreground)
                                .child("Where should the project open?"),
                        )
                        .child(
                            h_flex()
                                .gap_3()
                                .child(tile(
                                    "place-this-window",
                                    "app-window",
                                    "This Window",
                                    format!("Closes {current_name} and opens {name} instead"),
                                    Placement::Replace(current),
                                ))
                                .child(tile(
                                    "place-new-tab",
                                    "panels-top-left",
                                    "New Tab",
                                    format!("Keeps {current_name} open in a tab next to it"),
                                    Placement::NewTab,
                                )),
                        )
                        .child(
                            Checkbox::new("remember-placement")
                                .label("Remember my choice (change it in Settings → Appearance)")
                                .checked(*remember.read(cx))
                                .on_click(move |checked: &bool, _, cx| {
                                    remember_toggle.update(cx, |r, cx| {
                                        *r = *checked;
                                        cx.notify();
                                    });
                                }),
                        ),
                )
                .footer(
                    h_flex().w_full().justify_end().child(
                        Button::new("cancel")
                            .ghost()
                            .small()
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
                )
        });
    });
    if result.is_err() {
        // The window vanished meanwhile; open the project on its own.
        open_window(fallback, Placement::NewWindow, cx);
    }
}

fn remember_placement(placement: Placement, cx: &mut App) {
    cx.global_mut::<AppConfig>().0.open_project_mode = match placement {
        Placement::NewTab => OpenProjectMode::NewTab,
        _ => OpenProjectMode::ThisWindow,
    };
    crate::save_config(cx);
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
    // The merge runs asynchronously on the main queue; redraw the tab strips once it is done.
    cx.spawn(async move |cx| {
        cx.background_executor()
            .timer(std::time::Duration::from_millis(150))
            .await;
        cx.update(|cx| cx.refresh_windows());
    })
    .detach();
}
