//! Settings → Claude Code: the IDE's Claude settings file and the `claude` binary, loaded in the
//! background and reloaded when the file changes, so editing by hand is as good as in the UI.

mod folder;
mod launch;
mod page;

use std::path::PathBuf;
use std::time::Duration;

use claude_integration::settings::Loaded;
use claude_integration::{ClaudeDir, ClaudeSettings, launcher, legacy};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;
use notify::Watcher as _;

use crate::config::Config;
use crate::inbox::Inbox;

pub use launch::plan_for;
pub use page::page;

/// Changes to the folder are collected this long before one reload.
const DEBOUNCE: Duration = Duration::from_millis(300);

/// The IDE's Claude settings, plus what was found about `claude` itself.
#[derive(Debug, Clone)]
pub struct ClaudeState {
    pub dir: ClaudeDir,
    /// The last valid settings (defaults until a valid file is read).
    pub settings: ClaudeSettings,
    /// Why `settings.toml` was rejected, shown inline; the last valid settings stay active.
    pub settings_error: Option<String>,
    /// Keys of removed features still in `settings.toml`; ignored until the file is saved again.
    pub removed_keys: Vec<String>,
    /// What the one-time cleanup took out of `~/.claude` in this run.
    pub cleaned_up: Vec<String>,
    pub binary: Binary,
}

/// The `claude` executable as detected.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Binary {
    #[default]
    Detecting,
    Found {
        path: PathBuf,
        version: String,
    },
    Missing(String),
}

impl Global for ClaudeState {}

/// A fresh read of the settings file (done off the UI thread).
type Snapshot = Result<Loaded, String>;

pub fn claude_dir() -> Option<ClaudeDir> {
    Config::dir().map(|dir| ClaudeDir::new(dir.join("claude")))
}

/// `~/.claude`, honoring `CLAUDE_CONFIG_DIR` like Claude Code does.
pub fn claude_home() -> Option<PathBuf> {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".claude")))
}

/// Generated files for a project (session settings, MCP config):
/// `<cache>/power-idea/claude/<project key>`.
pub fn cache_root() -> Option<PathBuf> {
    dirs::cache_dir().map(|dir| dir.join("power-idea").join("claude"))
}

fn read_snapshot(dir: &ClaudeDir) -> Snapshot {
    dir.load_settings().map_err(|e| format!("{e:#}"))
}

/// Creates the folder (schema), loads it, watches it, detects `claude` and runs the one-time
/// cleanup of what the removed Install mode put into `~/.claude`.
pub fn init(cx: &mut App) {
    let Some(dir) = claude_dir() else { return };
    cx.set_global(ClaudeState {
        dir: dir.clone(),
        settings: ClaudeSettings::default(),
        settings_error: None,
        removed_keys: Vec::new(),
        cleaned_up: Vec::new(),
        binary: Binary::Detecting,
    });
    let setup_dir = dir.clone();
    cx.spawn(async move |cx| {
        let (snapshot, cleaned_up) = cx
            .background_spawn(async move {
                if let Err(err) = setup_dir.ensure_layout() {
                    eprintln!("Claude folder: {err:#}");
                }
                let cleaned_up = claude_home().map_or_default(|home| {
                    legacy::cleanup(&home, &setup_dir.root).unwrap_or_else(|err| {
                        error_journal::report(&err, "cleaning up the old Claude Code install");
                        Vec::new()
                    })
                });
                (read_snapshot(&setup_dir), cleaned_up)
            })
            .await;
        cx.update(|cx| {
            apply(snapshot, cx);
            watch(dir, cx);
            detect_binary(cx);
            announce_cleanup(cleaned_up, cx);
        });
    })
    .detach();
}

/// One notice for what the cleanup removed; the settings page keeps showing it for this run.
fn announce_cleanup(cleaned_up: Vec<String>, cx: &mut App) {
    if cleaned_up.is_empty() {
        return;
    }
    let text = cleanup_notice(&cleaned_up);
    cx.global_mut::<ClaudeState>().cleaned_up = cleaned_up;
    if let Some(window) = cx.active_window() {
        let _ = window.update(cx, |_, window, cx| {
            let notice = Notification::info(text).title("Claude Code: old IDE additions removed");
            window.push_notification(notice, cx);
        });
    }
}

pub(crate) fn cleanup_notice(removed: &[String]) -> String {
    format!(
        "Removed what Power IDEA once installed: {}. Your own files were not touched.",
        removed.join(", ")
    )
}

fn apply(snapshot: Snapshot, cx: &mut App) {
    let state = cx.global_mut::<ClaudeState>();
    match snapshot {
        Ok(loaded) => {
            state.settings = loaded.settings;
            state.removed_keys = loaded.removed_keys;
            state.settings_error = None;
        }
        Err(err) => state.settings_error = Some(err),
    }
    cx.refresh_windows();
}

/// Reloads the folder in the background.
pub fn reload(cx: &mut App) {
    let dir = cx.global::<ClaudeState>().dir.clone();
    cx.spawn(async move |cx| {
        let snapshot = cx.background_spawn(async move { read_snapshot(&dir) }).await;
        cx.update(|cx| apply(snapshot, cx));
    })
    .detach();
}

fn watch(dir: ClaudeDir, cx: &mut App) {
    let events: Inbox<()> = Inbox::default();
    let sender = events.clone();
    let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if event.is_ok_and(|e| !e.kind.is_access()) {
            sender.push(());
        }
    });
    let mut watcher = match watcher {
        Ok(watcher) => watcher,
        Err(err) => return eprintln!("Claude folder watcher unavailable: {err}"),
    };
    if let Err(err) = watcher.watch(&dir.root, notify::RecursiveMode::Recursive) {
        return eprintln!("cannot watch {}: {err}", dir.root.display());
    }
    cx.spawn(async move |cx| {
        // The watcher lives as long as this loop.
        let _watcher = watcher;
        while events.next_batch().await.is_some() {
            cx.background_executor().timer(DEBOUNCE).await;
            events.take_all();
            cx.update(reload);
        }
    })
    .detach();
}

/// Finds `claude` (configured path, PATH, usual places) and asks for its version.
pub fn detect_binary(cx: &mut App) {
    let configured = cx.global::<ClaudeState>().settings.general.claude_path.clone();
    cx.global_mut::<ClaudeState>().binary = Binary::Detecting;
    cx.spawn(async move |cx| {
        let binary = cx
            .background_spawn(async move {
                let home = dirs::home_dir().unwrap_or_default();
                let path_env = std::env::var("PATH").ok();
                let Some(path) = launcher::locate_claude(&configured, path_env.as_deref(), &home) else {
                    return Binary::Missing(if configured.trim().is_empty() {
                        "claude was not found on PATH".into()
                    } else {
                        format!("{configured} does not exist")
                    });
                };
                match std::process::Command::new(&path).arg("--version").output() {
                    Ok(out) if out.status.success() => Binary::Found {
                        version: String::from_utf8_lossy(&out.stdout).trim().to_string(),
                        path,
                    },
                    Ok(out) => Binary::Missing(format!(
                        "{} --version failed: {}",
                        path.display(),
                        String::from_utf8_lossy(&out.stderr).trim()
                    )),
                    Err(err) => Binary::Missing(format!("{}: {err}", path.display())),
                }
            })
            .await;
        cx.update(|cx| {
            cx.global_mut::<ClaudeState>().binary = binary;
            cx.refresh_windows();
        });
    })
    .detach();
}

/// Applies a settings change, saves `settings.toml` (the watcher then confirms it) and redraws.
pub fn update_settings(cx: &mut App, update: impl FnOnce(&mut ClaudeSettings)) {
    let state = cx.global_mut::<ClaudeState>();
    let mut settings = state.settings.clone();
    update(&mut settings);
    if settings == state.settings {
        return;
    }
    if let Err(err) = state.dir.save_settings(&settings) {
        state.settings_error = Some(format!("{err:#}"));
    } else {
        state.settings = settings;
        state.settings_error = None;
        // The file was just written without them.
        state.removed_keys.clear();
    }
    cx.refresh_windows();
}

pub fn state(cx: &App) -> &ClaudeState {
    cx.global::<ClaudeState>()
}
