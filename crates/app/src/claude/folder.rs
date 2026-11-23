//! Settings → Claude Code → Claude folder: opens Claude Code's own files (`~/.claude`) for
//! editing. A missing file is created, empty, only when its button is clicked.

use std::path::{Path, PathBuf};

use claude_integration::home;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::setting::{SettingGroup, SettingItem};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::*;

use super::claude_home;
use crate::style::dialog_button;
use crate::workspace::Workspace;

pub(super) fn group(workspace: WeakEntity<Workspace>) -> SettingGroup {
    SettingGroup::new()
        .title("Claude folder")
        .description(
            "Claude Code's own files. Model, permissions and hooks go in settings.json; instructions for every \
             project in CLAUDE.md. The IDE never writes into them.",
        )
        .item(SettingItem::render(move |_, _, cx| buttons(&workspace, cx)))
}

fn buttons(workspace: &WeakEntity<Workspace>, cx: &App) -> AnyElement {
    let (settings_ws, rules_ws) = (workspace.clone(), workspace.clone());
    let location = claude_home().map_or_else(|| "~/.claude".to_string(), |home| tilde(&home));
    v_flex()
        .gap_2()
        .child(
            h_flex()
                .gap_2()
                .flex_wrap()
                .child(
                    dialog_button("claude-edit-settings")
                        .label("Edit Claude settings")
                        .on_click(move |_, window, cx| edit(&settings_ws, home::user_settings, window, cx)),
                )
                .child(
                    dialog_button("claude-edit-rules")
                        .label("Edit global rules")
                        .on_click(move |_, window, cx| edit(&rules_ws, home::global_rules, window, cx)),
                )
                .child(
                    dialog_button("claude-open-home")
                        .label("Open ~/.claude")
                        .on_click(|_, window, cx| open_home(window, cx)),
                ),
        )
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(format!("{location}/settings.json · {location}/CLAUDE.md")),
        )
        .into_any_element()
}

/// Opens the file `file_of(~/.claude)` in an editor tab, creating it first when missing.
fn edit(workspace: &WeakEntity<Workspace>, file_of: fn(&Path) -> PathBuf, window: &mut Window, cx: &mut App) {
    let Some(path) = claude_home().map(|home| file_of(&home)) else {
        return notify_error("Cannot find the home folder.".into(), window, cx);
    };
    if let Err(err) = home::ensure_file(&path) {
        return notify_error(format!("{err:#}"), window, cx);
    }
    workspace
        .update(cx, |ws, cx| ws.open_path(&path, None, window, cx))
        .ok();
}

fn open_home(window: &mut Window, cx: &mut App) {
    let Some(dir) = claude_home() else { return };
    if !dir.is_dir() {
        return notify_error(
            format!("{} does not exist yet: start claude once.", dir.display()),
            window,
            cx,
        );
    }
    if let Err(err) = std::process::Command::new("open").arg(&dir).spawn() {
        notify_error(format!("Cannot open {}: {err}", dir.display()), window, cx);
    }
}

fn notify_error(message: String, window: &mut Window, cx: &mut App) {
    window.push_notification(Notification::error(message), cx);
}

/// `path` with the home folder shown as `~`.
fn tilde(path: &Path) -> String {
    let home = dirs::home_dir().unwrap_or_default();
    match path.strip_prefix(&home) {
        Ok(rest) if !home.as_os_str().is_empty() => format!("~/{}", rest.display()),
        _ => path.display().to_string(),
    }
}
