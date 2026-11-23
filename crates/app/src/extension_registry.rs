//! Installed extensions (`<config>/extensions/*/extension.toml`): discovery, their commands
//! in Find Action and the keymap, and reloading. Processes run per project window
//! (`workspace/extensions.rs`).

use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;
use std::sync::Arc;

use ::extensions::Extension;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;

use crate::actions::{self, RunExtensionCommand, WORKSPACE_CONTEXT};
use crate::config::Config;
use crate::workspace::Workspaces;

#[derive(Default)]
pub struct ExtensionRegistry {
    pub extensions: Vec<Arc<Extension>>,
    /// Folders that could not be loaded, not yet shown to the user.
    unreported_errors: Vec<String>,
}

impl Global for ExtensionRegistry {}

pub fn dir() -> Option<PathBuf> {
    Config::dir().map(|dir| dir.join("extensions"))
}

/// Where an extension's stderr goes: `~/Library/Logs/Power IDEA/extensions/<id>.log`.
pub fn log_path(id: &str) -> Option<PathBuf> {
    dirs::home_dir().map(|home| {
        home.join("Library/Logs/Power IDEA/extensions")
            .join(format!("{id}.log"))
    })
}

/// Discovers the installed extensions; call before `actions::bind_keys`.
pub fn load(cx: &mut App) {
    let found = dir().map_or_default(|dir| ::extensions::discover(&dir));
    for error in &found.errors {
        eprintln!("extension {error}");
    }
    cx.set_global(ExtensionRegistry {
        extensions: found.extensions.into_iter().map(Arc::new).collect(),
        unreported_errors: found.errors,
    });
}

pub fn installed(cx: &App) -> Vec<Arc<Extension>> {
    cx.try_global::<ExtensionRegistry>()
        .map_or_default(|registry| registry.extensions.clone())
}

/// Load errors not shown yet (each is shown once, in the first window that asks).
pub fn take_errors(cx: &mut App) -> Vec<String> {
    std::mem::take(&mut cx.default_global::<ExtensionRegistry>().unreported_errors)
}

/// The extension that declares command `id`.
pub fn owner_of(command: &str, cx: &App) -> Option<Arc<Extension>> {
    installed(cx)
        .into_iter()
        .find(|ext| ext.command(command).is_some())
}

/// Default bindings of extension commands, active in project windows.
pub fn key_bindings(cx: &App) -> Vec<KeyBinding> {
    let context = KeyBindingContextPredicate::parse(WORKSPACE_CONTEXT)
        .ok()
        .map(Rc::new);
    installed(cx)
        .iter()
        .flat_map(|ext| {
            ext.manifest
                .commands
                .iter()
                .map(move |c| (ext.id().to_string(), c))
        })
        .filter_map(|(ext, command)| {
            let keys = command.keybinding.as_deref()?;
            let action = Box::new(RunExtensionCommand {
                id: command.id.clone().into(),
            });
            KeyBinding::load(keys, action, context.clone(), false, None, cx)
                .map_err(|err| eprintln!("extension {ext}: bad keybinding {keys}: {err}"))
                .ok()
        })
        .collect()
}

/// Find Action rows: "<extension name>: <command title>".
pub fn palette_entries(cx: &App) -> Vec<(SharedString, Box<dyn Action>)> {
    installed(cx)
        .iter()
        .flat_map(|ext| {
            ext.manifest.commands.iter().map(|command| {
                let label: SharedString = format!("{}: {}", ext.name(), command.title).into();
                let action: Box<dyn Action> = Box::new(RunExtensionCommand {
                    id: command.id.clone().into(),
                });
                (label, action)
            })
        })
        .collect()
}

/// Reload Extensions: discovers again, rebinds keys and restarts every window's processes.
pub fn reload(cx: &mut App) {
    load(cx);
    actions::bind_keys(cx);
    let count = installed(cx).len();
    let errors = take_errors(cx);
    for (handle, workspace) in Workspaces::live(cx) {
        let _ = handle.update(cx, |_, window, cx| {
            workspace.update(cx, |ws, cx| ws.restart_extensions(window, cx));
        });
    }
    let Some(window) = cx.active_window() else { return };
    let _ = window.update(cx, |_, window, cx| {
        let note = format!("{count} extension{} loaded", if count == 1 { "" } else { "s" });
        window.push_notification(Notification::success(note).title("Extensions reloaded"), cx);
        if !errors.is_empty() {
            window.push_notification(errors_notification(&errors), cx);
        }
    });
}

pub fn errors_notification(errors: &[String]) -> Notification {
    Notification::error(errors.join("\n")).title("Some extensions could not be loaded")
}

/// Open Extensions Folder: creates it when missing and shows it in Finder.
pub fn open_folder(window: &mut Window, cx: &mut App) {
    let Some(dir) = dir() else { return };
    let result =
        std::fs::create_dir_all(&dir).and_then(|()| Command::new("open").arg(&dir).spawn().map(drop));
    if let Err(err) = result {
        let message = format!("Cannot open {}: {err}", dir.display());
        window.push_notification(Notification::error(message), cx);
    }
}
