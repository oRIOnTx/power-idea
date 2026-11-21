mod actions;
mod config;
mod editor;
mod mcp_bridge;
mod palette;
mod project_tree;
mod style;
mod terminal;
mod workspace;

use std::path::PathBuf;

use gpui_kit::component::TitleBar;
use std::rc::Rc;

use gpui_kit::component::theme::{Theme, ThemeConfig, ThemeMode, ThemeSet};
use gpui_kit::*;

use crate::actions::{OpenProject, Quit, ToggleTheme};
use crate::config::{AppConfig, Appearance, Config};
use crate::workspace::Workspace;

/// Bundled "Neo" theme set (dark + light).
const NEO_THEME: &str = include_str!("../themes/neo.json");

/// Preferred editor fonts, tried in order after the configured one.
const MONO_FONT_FALLBACKS: &[&str] = &[
    "JetBrains Mono",
    "JetBrainsMono Nerd Font",
    "JetBrainsMonoNL Nerd Font",
    "SF Mono",
    "Menlo",
];

fn neo_themes() -> Vec<Rc<ThemeConfig>> {
    match serde_json::from_str::<ThemeSet>(NEO_THEME) {
        Ok(set) => set.themes.into_iter().map(Rc::new).collect(),
        Err(err) => {
            eprintln!("bundled theme is invalid: {err}");
            Vec::new()
        }
    }
}

fn apply_theme(window: Option<&mut Window>, cx: &mut App) {
    let config = cx.global::<AppConfig>().0.clone();
    let dark = config.appearance == Appearance::Dark;
    let mode = if dark { ThemeMode::Dark } else { ThemeMode::Light };
    Theme::change(mode, window, cx);

    // Install the other mode first so the active one is applied last.
    let mut themes = neo_themes();
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

fn save_config(cx: &App) {
    if let Err(err) = cx.global::<AppConfig>().0.save() {
        eprintln!("failed to save config: {err:#}");
    }
}

/// Opens `path` in its own window, or focuses the window that already shows it.
pub fn open_project(path: PathBuf, cx: &mut App) {
    let path = path.canonicalize().unwrap_or(path);
    let existing = workspace::Workspaces::live(cx)
        .into_iter()
        .find(|(_, ws)| ws.read(cx).root() == &path);
    if let Some((handle, _)) = existing {
        handle.update(cx, |_, window, _| window.activate_window()).ok();
        return;
    }

    cx.global_mut::<AppConfig>().0.touch_recent_project(&path);
    save_config(cx);

    let bounds = Bounds::centered(None, size(px(1440.), px(900.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(640.), px(400.))),
        ..TitleBar::window_options()
    };
    let result = gpui_kit::open_window(options, cx, |window, cx| {
        window.set_window_title(&path.file_name().unwrap_or_default().to_string_lossy());
        cx.new(|cx| Workspace::new(path.clone(), window, cx))
    });
    if let Err(err) = result {
        eprintln!("failed to open window: {err:#}");
    }
}

fn prompt_open_project(cx: &mut App) {
    let paths = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some("Open Project".into()),
    });
    cx.spawn(async move |cx| {
        match paths.await {
            Ok(Ok(Some(mut paths))) if !paths.is_empty() => {
                cx.update(|cx| open_project(paths.remove(0), cx));
            }
            // Nothing to show if the user cancels on startup.
            _ => cx.update(|cx| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            }),
        }
    })
    .detach();
}

fn main() {
    let arg_project = std::env::args().nth(1).map(PathBuf::from);

    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            cx.set_global(AppConfig(Config::load()));
            apply_theme(None, cx);
            actions::bind_keys(cx);
            mcp_bridge::start(cx);

            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.on_action(|_: &OpenProject, cx| prompt_open_project(cx));
            cx.on_action(|_: &ToggleTheme, cx| {
                let config = &mut cx.global_mut::<AppConfig>().0;
                config.appearance = match config.appearance {
                    Appearance::Dark => Appearance::Light,
                    Appearance::Light => Appearance::Dark,
                };
                save_config(cx);
                apply_theme(None, cx);
                cx.refresh_windows();
            });
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            let last_project = cx.global::<AppConfig>().0.recent_projects.first().cloned();
            match arg_project.or(last_project).filter(|p| p.is_dir()) {
                Some(path) => open_project(path, cx),
                None => prompt_open_project(cx),
            }
            cx.activate(true);
        });
}

#[cfg(test)]
mod tests {
    use super::neo_themes;

    #[test]
    fn bundled_theme_parses() {
        let themes = neo_themes();
        assert_eq!(themes.len(), 2);
        assert!(themes.iter().any(|t| t.mode.is_dark()));
        assert!(themes.iter().all(|t| t.highlight.is_some()));
    }
}
