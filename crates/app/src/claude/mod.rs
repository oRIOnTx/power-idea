//! Settings → Claude Code: the global state of the IDE's Claude folder (settings, rules,
//! skills, the `claude` binary), loaded in the background and reloaded when files change, so
//! editing by hand or by an agent is as good as editing in the UI.

mod page;
mod preview;
mod rule_actions;
mod rules_view;
mod skills_view;

use std::path::PathBuf;
use std::time::Duration;

use claude_integration::installer::{Installer, Manifest};
use claude_integration::rules::LoadError;
use claude_integration::skills::{self, SkillError};
use claude_integration::{ClaudeDir, ClaudeSettings, Rule, Skill, launcher};
use gpui_kit::*;
use notify::Watcher as _;

use crate::config::Config;
use crate::inbox::Inbox;

pub use page::page;
pub use preview::open_preview;

/// Changes to the folder are collected this long before one reload.
const DEBOUNCE: Duration = Duration::from_millis(300);

/// Everything read from the Claude folder, plus what was found about `claude` itself.
#[derive(Debug, Clone)]
pub struct ClaudeState {
    pub dir: ClaudeDir,
    /// The last valid settings (defaults until a valid file is read).
    pub settings: ClaudeSettings,
    /// Why `settings.toml` was rejected, shown inline; the last valid settings stay active.
    pub settings_error: Option<String>,
    pub rules: Vec<Rule>,
    pub rule_errors: Vec<LoadError>,
    pub skills: Vec<Skill>,
    pub skill_errors: Vec<SkillError>,
    pub binary: Binary,
    pub manifest: Manifest,
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

/// A fresh read of the folder (done off the UI thread).
struct Snapshot {
    settings: Result<ClaudeSettings, String>,
    rules: (Vec<Rule>, Vec<LoadError>),
    skills: (Vec<Skill>, Vec<SkillError>),
    manifest: Manifest,
}

pub fn claude_dir() -> Option<ClaudeDir> {
    Config::dir().map(|dir| ClaudeDir::new(dir.join("claude")))
}

/// `~/.claude`, honoring `CLAUDE_CONFIG_DIR` like Claude Code does.
pub fn claude_home() -> Option<PathBuf> {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".claude")))
}

/// The Unix socket the hook / status-line helpers write to (in the per-user temp folder:
/// socket paths are limited to ~100 bytes).
pub fn bridge_socket() -> PathBuf {
    std::env::temp_dir().join("power-idea-claude.sock")
}

/// Generated files for a project: `<cache>/power-idea/claude/<project key>`.
pub fn cache_root() -> Option<PathBuf> {
    dirs::cache_dir().map(|dir| dir.join("power-idea").join("claude"))
}

fn installer(dir: &ClaudeDir) -> Option<Installer> {
    claude_home().map(|home| Installer::new(home, dir.root.clone()))
}

fn read_snapshot(dir: &ClaudeDir) -> Snapshot {
    let manifest = installer(dir).map_or_default(|i| i.manifest());
    Snapshot {
        settings: dir.load_settings().map_err(|e| format!("{e:#}")),
        rules: claude_integration::rules::load_dir(&dir.rules()),
        skills: skills::load_dir(&dir.skills()),
        manifest,
    }
}

/// Creates the folder (schemas, built-in skills), loads it, watches it and detects `claude`.
pub fn init(cx: &mut App) {
    let Some(dir) = claude_dir() else { return };
    cx.set_global(ClaudeState {
        dir: dir.clone(),
        settings: ClaudeSettings::default(),
        settings_error: None,
        rules: Vec::new(),
        rule_errors: Vec::new(),
        skills: Vec::new(),
        skill_errors: Vec::new(),
        binary: Binary::Detecting,
        manifest: Manifest::default(),
    });
    let setup_dir = dir.clone();
    cx.spawn(async move |cx| {
        let snapshot = cx
            .background_spawn(async move {
                if let Err(err) = setup_dir.ensure_layout() {
                    eprintln!("Claude folder: {err:#}");
                }
                if let Err(err) = skills::seed_builtins(&setup_dir.skills()) {
                    eprintln!("built-in skills: {err:#}");
                }
                read_snapshot(&setup_dir)
            })
            .await;
        cx.update(|cx| {
            apply(snapshot, cx);
            watch(dir, cx);
            detect_binary(cx);
        });
    })
    .detach();
}

fn apply(snapshot: Snapshot, cx: &mut App) {
    let state = cx.global_mut::<ClaudeState>();
    match snapshot.settings {
        Ok(settings) => {
            state.settings = settings;
            state.settings_error = None;
        }
        Err(err) => state.settings_error = Some(err),
    }
    (state.rules, state.rule_errors) = snapshot.rules;
    (state.skills, state.skill_errors) = snapshot.skills;
    state.manifest = snapshot.manifest;
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
    }
    cx.refresh_windows();
}

pub fn state(cx: &App) -> &ClaudeState {
    cx.global::<ClaudeState>()
}
