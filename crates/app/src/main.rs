mod actions;
mod assets;
mod branches_popup;
mod config;
mod database;
mod editor;
mod file_icons;
mod file_ops;
mod inbox;
mod language;
mod mcp_bridge;
mod menus;
mod palette;
mod preview;
mod project_tree;
mod projects;
mod recovery;
mod session;
mod settings;
mod style;
mod terminal;
mod tree_selection;
mod usages;
mod watcher;
mod welcome;
mod word_completion;
mod workspace;

use std::path::PathBuf;

use std::rc::Rc;

use gpui_kit::component::theme::{Theme, ThemeConfig, ThemeMode, ThemeSet};
use gpui_kit::*;

use crate::actions::{OpenProject, Quit, ToggleTheme};
use crate::config::{AppConfig, Appearance, Config};

/// Version shown to the user, e.g. `0.2.0 (build 61, 5d0e08f)`; see `build.rs`.
pub(crate) const APP_VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (build ",
    env!("POWER_IDEA_BUILD"),
    ", ",
    env!("POWER_IDEA_COMMIT"),
    ")"
);

/// Bundled theme sets: "Neo" (dark + light) and "Graphite" (a lighter dark).
const NEO_THEME: &str = include_str!("../themes/neo.json");
const GRAPHITE_THEME: &str = include_str!("../themes/graphite.json");

/// Preferred editor fonts, tried in order after the configured one.
const MONO_FONT_FALLBACKS: &[&str] = &[
    "JetBrains Mono",
    "JetBrainsMono Nerd Font",
    "JetBrainsMonoNL Nerd Font",
    "SF Mono",
    "Menlo",
];

fn parse_themes(json: &str) -> Vec<Rc<ThemeConfig>> {
    match serde_json::from_str::<ThemeSet>(json) {
        Ok(set) => set.themes.into_iter().map(Rc::new).collect(),
        Err(err) => {
            eprintln!("bundled theme is invalid: {err}");
            Vec::new()
        }
    }
}

fn neo_themes() -> Vec<Rc<ThemeConfig>> {
    parse_themes(NEO_THEME)
}

/// The light theme and the dark theme that `appearance` uses.
fn themes_for(appearance: Appearance) -> Vec<Rc<ThemeConfig>> {
    let mut themes = neo_themes();
    if appearance == Appearance::Graphite {
        themes.retain(|t| !t.mode.is_dark());
        themes.extend(parse_themes(GRAPHITE_THEME));
    }
    themes
}

pub(crate) fn apply_theme(window: Option<&mut Window>, cx: &mut App) {
    let config = cx.global::<AppConfig>().0.clone();
    let dark = config.appearance.is_dark();
    let mode = if dark { ThemeMode::Dark } else { ThemeMode::Light };
    Theme::change(mode, window, cx);

    // Install the other mode first so the active one is applied last.
    let mut themes = themes_for(config.appearance);
    themes.sort_by_key(|t| t.mode.is_dark() == dark);
    Theme::update(cx, |theme| {
        for config in &themes {
            theme.apply_config(config);
        }
    });

    let installed = cx.text_system().all_font_names();
    let mono_font = std::iter::once(config.editor_font_family.as_str())
        .chain(MONO_FONT_FALLBACKS.iter().copied())
        .find(|name| installed.iter().any(|f| f == name))
        .map(SharedString::from);
    Theme::update(cx, |theme| {
        if let Some(font) = mono_font {
            theme.mono_font_family = font;
        }
        theme.mono_font_size = px(config.editor_font_size);
        theme.font_size = px(config.ui_font_size);
    });
}

pub(crate) fn save_config(cx: &App) {
    if let Err(err) = cx.global::<AppConfig>().0.save() {
        eprintln!("failed to save config: {err:#}");
    }
}

/// GUI apps started from Finder get a minimal PATH. Ask the user's login
/// shell for its PATH so git, language servers and tools from Homebrew,
/// rbenv, nvm, etc. are found, like JetBrains IDEs and VS Code do.
#[allow(unsafe_code)] // `set_var` is unsafe in edition 2024; see the SAFETY note.
fn inherit_login_shell_path() {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let Ok(mut child) = Command::new(shell)
        .args(["-l", "-c", "printf '%s' \"$PATH\""])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return;
    };
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => return,
        }
    }
    if !matches!(child.try_wait(), Ok(Some(status)) if status.success()) {
        let _ = child.kill();
        return;
    }
    let Ok(output) = child.wait_with_output() else {
        return;
    };
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !path.is_empty() {
        // SAFETY: called at the very start of `main`, before any other thread exists.
        unsafe { std::env::set_var("PATH", path) };
    }
}

/// Appends panics (message + source location) to
/// `~/Library/Logs/Power IDEA/panic.log`. Panics inside AppKit callbacks abort
/// the process, so this file is the only trace a crash leaves behind.
fn install_panic_log() {
    let Some(dir) = dirs::home_dir().map(|home| home.join("Library/Logs/Power IDEA")) else {
        return;
    };
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        use std::io::Write as _;
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("panic.log"))
        {
            let thread = std::thread::current();
            let backtrace = std::backtrace::Backtrace::force_capture();
            let _ = writeln!(
                file,
                "--- {:?} thread '{}' v{}\n{info}\n{backtrace}",
                std::time::SystemTime::now(),
                thread.name().unwrap_or("?"),
                APP_VERSION,
            );
        }
        default_hook(info);
    }));
}

fn main() {
    install_panic_log();
    inherit_login_shell_path();
    let arg_project = std::env::args().nth(1).map(PathBuf::from);

    gpui_kit::application()
        .with_assets(assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            cx.set_global(AppConfig(Config::load()));
            apply_theme(None, cx);
            actions::bind_keys(cx);
            mcp_bridge::start(cx);
            menus::install(cx);

            cx.on_action(|_: &Quit, cx| workspace::request_quit(cx));
            cx.on_app_quit(|cx| {
                projects::remember_open_projects(cx);
                workspace::store_recovery(cx);
                async {}
            })
            .detach();
            cx.on_action(|_: &OpenProject, cx| projects::prompt_open_project(cx));
            cx.on_action(|_: &ToggleTheme, cx| {
                let config = &mut cx.global_mut::<AppConfig>().0;
                config.appearance = match config.appearance {
                    Appearance::Dark | Appearance::Graphite => Appearance::Light,
                    Appearance::Light => Appearance::Dark,
                };
                save_config(cx);
                apply_theme(None, cx);
                cx.refresh_windows();
            });
            // Like JetBrains: closing the last project shows the Welcome screen;
            // closing the Welcome screen itself quits.
            cx.on_window_closed(|cx, id| {
                let was_welcome = welcome::forget_closed(id, cx);
                // Remaining project tabs redraw their tab strip.
                cx.refresh_windows();
                if cx.windows().is_empty() {
                    if was_welcome {
                        cx.quit();
                    } else {
                        welcome::show(cx);
                    }
                }
            })
            .detach();

            // Not deferred: nothing would flush a deferred call during launch.
            match arg_project.filter(|p| p.is_dir()) {
                Some(path) => projects::open_project_now(path, cx),
                None => {
                    let mut paths = cx.global::<AppConfig>().0.startup_projects();
                    paths.retain(|p| p.is_dir());
                    if paths.is_empty() {
                        welcome::show(cx);
                    } else {
                        projects::reopen_projects(paths, cx);
                    }
                }
            }
            cx.activate(true);
        });
}

#[cfg(test)]
mod tests {
    use super::{neo_themes, themes_for};
    use crate::config::Appearance;

    #[test]
    fn bundled_theme_parses() {
        let themes = neo_themes();
        assert_eq!(themes.len(), 2);
        assert!(themes.iter().any(|t| t.mode.is_dark()));
        assert!(themes.iter().all(|t| t.highlight.is_some()));
    }

    #[test]
    fn graphite_replaces_the_dark_theme() {
        let themes = themes_for(Appearance::Graphite);
        let dark: Vec<_> = themes
            .iter()
            .filter(|t| t.mode.is_dark())
            .map(|t| t.name.to_string())
            .collect();
        assert_eq!(dark, ["Graphite"]);
        assert!(themes.iter().any(|t| !t.mode.is_dark()));
        assert!(themes.iter().all(|t| t.highlight.is_some()));
    }
}
