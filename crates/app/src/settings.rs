//! Settings tab (Cmd+,): IDE preferences, language servers, MCP, keymap and
//! per-project settings. Every change is applied live and saved immediately.

use std::path::PathBuf;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::setting::{
    NumberFieldOptions, SettingField, SettingGroup, SettingItem, SettingPage, Settings,
};
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::actions::{self, WORKSPACE_CONTEXT};
use crate::config::{AppConfig, Appearance, Config, LanguageServerConfig};
use crate::projects::OpenProjectMode;
use crate::workspace::Workspace;

/// Applies a change to the global config, saves it and refreshes the UI.
pub fn update_config(cx: &mut App, update: impl FnOnce(&mut Config)) {
    update(&mut cx.global_mut::<AppConfig>().0);
    if let Err(err) = cx.global::<AppConfig>().0.save() {
        eprintln!("failed to save config: {err:#}");
    }
    crate::apply_theme(None, cx);
    cx.refresh_windows();
}

fn config(cx: &App) -> &Config {
    &cx.global::<AppConfig>().0
}

/// Absolute path of `command` if it is found in `PATH`.
pub fn find_in_path(command: &str) -> Option<PathBuf> {
    if command.contains('/') {
        let path = PathBuf::from(command);
        return path.is_file().then_some(path);
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(command))
        .find(|candidate| candidate.is_file())
}

fn monospace_fonts(cx: &App) -> Vec<(SharedString, SharedString)> {
    const HINTS: &[&str] = &[
        "Mono", "Code", "Menlo", "Monaco", "Courier", "Consolas", "Iosevka", "Hack", "Fira",
    ];
    let mut fonts: Vec<String> = cx
        .text_system()
        .all_font_names()
        .into_iter()
        .filter(|name| HINTS.iter().any(|hint| name.contains(hint)))
        .collect();
    fonts.sort();
    fonts.dedup();
    fonts.into_iter().map(|f| (f.clone().into(), f.into())).collect()
}

fn number(min: f64, max: f64) -> NumberFieldOptions {
    NumberFieldOptions { min, max, step: 1.0 }
}

pub fn render(workspace: WeakEntity<Workspace>, cx: &App) -> Settings {
    Settings::new("ide-settings").sidebar_width(px(220.)).pages([
        appearance_page(),
        editor_page(cx),
        terminal_page(),
        languages_page(cx),
        mcp_page(),
        insights_page(),
        keymap_page(workspace.clone()),
        project_page(workspace),
    ])
}

fn appearance_page() -> SettingPage {
    SettingPage::new("Appearance")
        .icon(IconName::Palette)
        .default_open(true)
        .group(
            SettingGroup::new()
                .item(
                    SettingItem::new(
                        "Theme",
                        SettingField::dropdown(
                            vec![
                                ("dark".into(), "Neo Dark".into()),
                                ("graphite".into(), "Graphite (JetBrains-style dark)".into()),
                                ("light".into(), "Neo Light".into()),
                            ],
                            |cx| match config(cx).appearance {
                                Appearance::Dark => "dark".into(),
                                Appearance::Graphite => "graphite".into(),
                                Appearance::Light => "light".into(),
                            },
                            |value, cx| {
                                update_config(cx, |c| {
                                    c.appearance = match value.as_ref() {
                                        "light" => Appearance::Light,
                                        "graphite" => Appearance::Graphite,
                                        _ => Appearance::Dark,
                                    }
                                });
                            },
                        )
                        .default_value("dark"),
                    )
                    .description("Also toggled from Find Action → Toggle Light/Dark Theme."),
                )
                .item(
                    SettingItem::new(
                        "Open projects in",
                        SettingField::dropdown(
                            vec![
                                ("ask".into(), "Ask every time".into()),
                                ("this_window".into(), "This window".into()),
                                ("new_tab".into(), "New tab".into()),
                            ],
                            |cx| match config(cx).open_project_mode {
                                OpenProjectMode::Ask => "ask".into(),
                                OpenProjectMode::ThisWindow => "this_window".into(),
                                OpenProjectMode::NewTab => "new_tab".into(),
                            },
                            |value, cx| {
                                update_config(cx, |c| {
                                    c.open_project_mode = match value.as_ref() {
                                        "this_window" => OpenProjectMode::ThisWindow,
                                        "new_tab" => OpenProjectMode::NewTab,
                                        _ => OpenProjectMode::Ask,
                                    };
                                });
                            },
                        )
                        .default_value("ask"),
                    )
                    .description("Projects open as native macOS window tabs."),
                )
                .item(SettingItem::new(
                    "Interface font size",
                    SettingField::number_input(
                        number(10., 20.),
                        |cx| config(cx).ui_font_size as f64,
                        |value, cx| update_config(cx, |c| c.ui_font_size = value as f32),
                    )
                    .default_value(13.),
                )),
        )
        .group(
            SettingGroup::new()
                .title("Confirmations")
                .item(
                    SettingItem::new(
                        "Confirm exit",
                        SettingField::switch(
                            |cx| config(cx).confirm_exit,
                            |value, cx| update_config(cx, |c| c.confirm_exit = value),
                        )
                        .default_value(true),
                    )
                    .description("Ask before ⌘Q quits. Unsaved files are always asked about."),
                )
                .item(
                    SettingItem::new(
                        "Confirm closing a project",
                        SettingField::switch(
                            |cx| config(cx).confirm_close_project,
                            |value, cx| update_config(cx, |c| c.confirm_close_project = value),
                        )
                        .default_value(true),
                    )
                    .description("Ask before ⇧⌘W, the close button or a project tab closes a project."),
                ),
        )
}

fn editor_page(cx: &App) -> SettingPage {
    let fonts = monospace_fonts(cx);
    SettingPage::new("Editor").icon(IconName::FileText).group(
        SettingGroup::new()
            .title("Font")
            .item(SettingItem::new(
                "Font family",
                SettingField::scrollable_dropdown(
                    fonts,
                    |cx| config(cx).editor_font_family.clone().into(),
                    |value, cx| update_config(cx, |c| c.editor_font_family = value.to_string()),
                ),
            ))
            .item(SettingItem::new(
                "Font size",
                SettingField::number_input(
                    number(8., 32.),
                    |cx| config(cx).editor_font_size as f64,
                    |value, cx| update_config(cx, |c| c.editor_font_size = value as f32),
                )
                .default_value(13.),
            ))
            .item(
                SettingItem::new(
                    "Tab size",
                    SettingField::number_input(
                        number(1., 16.),
                        |cx| config(cx).tab_size as f64,
                        |value, cx| update_config(cx, |c| c.tab_size = value as usize),
                    )
                    .default_value(4.),
                )
                .description("Applies to newly opened files. Projects can override it."),
            ),
    )
}

fn terminal_page() -> SettingPage {
    SettingPage::new("Terminal").icon(IconName::SquareTerminal).group(
        SettingGroup::new()
            .item(
                SettingItem::new(
                    "Shell",
                    SettingField::input(
                        |cx| config(cx).terminal_shell.clone().into(),
                        |value, cx| update_config(cx, |c| c.terminal_shell = value.trim().to_string()),
                    ),
                )
                .description("Path to the shell for new terminal tabs. Leave empty to use $SHELL."),
            )
            .item(
                SettingItem::new(
                    "Full window width",
                    SettingField::switch(
                        |cx| config(cx).bottom_panel_full_width,
                        |value, cx| update_config(cx, |c| c.bottom_panel_full_width = value),
                    )
                    .default_value(true),
                )
                .description(
                    "The terminal and other bottom tool windows span the whole window, \
                     below the Project tree, as in JetBrains IDEs.",
                ),
            ),
    )
}

fn server_status(server: &LanguageServerConfig, cx: &App) -> AnyElement {
    let theme = cx.theme();
    match find_in_path(&server.command) {
        Some(path) => h_flex()
            .gap_2()
            .text_xs()
            .child(crate::style::signal_dot(theme.green))
            .child(
                div()
                    .text_color(theme.muted_foreground)
                    .child(format!("Installed · {}", path.display())),
            )
            .into_any_element(),
        None => {
            let hint = server.install_hint.clone();
            h_flex()
                .gap_2()
                .text_xs()
                .child(div().size(px(6.)).rounded_full().bg(theme.warning))
                .child(div().text_color(theme.muted_foreground).child("Not installed"))
                .when(!hint.is_empty(), |el| {
                    el.child(
                        div()
                            .px_1p5()
                            .rounded_sm()
                            .bg(theme.muted)
                            .font_family(theme.mono_font_family.clone())
                            .child(hint.clone()),
                    )
                    .child(
                        Button::new(SharedString::from(format!("copy-{hint}")))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Copy)
                            .tooltip("Copy install command")
                            .on_click(move |_, window, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(hint.clone()));
                                window.push_notification(Notification::success("Install command copied"), cx);
                            }),
                    )
                })
                .into_any_element()
        }
    }
}

/// Getter/setter pair bound to one field of the language server `id`.
fn server_field<T: Default + 'static>(
    id: &str,
    get: impl Fn(&LanguageServerConfig) -> T + 'static,
    set: impl Fn(&mut LanguageServerConfig, T) + 'static,
) -> (impl Fn(&App) -> T + 'static, impl Fn(T, &mut App) + 'static) {
    let (get_id, set_id) = (id.to_string(), id.to_string());
    let getter = move |cx: &App| config(cx).language_servers.get(&get_id).map_or_default(&get);
    let setter = move |value: T, cx: &mut App| {
        update_config(cx, |c| {
            if let Some(server) = c.language_servers.get_mut(&set_id) {
                set(server, value);
            }
        });
    };
    (getter, setter)
}

fn title_case(id: &str) -> String {
    let mut chars = id.chars();
    chars
        .next()
        .map_or_default(|c| c.to_uppercase().chain(chars).collect())
}

fn language_group(id: String) -> SettingGroup {
    let (enabled, set_enabled) = server_field(&id, |s| s.enabled, |s, v| s.enabled = v);
    let (command, set_command) = server_field(
        &id,
        |s| SharedString::from(s.command.clone()),
        |s, v| s.command = v.trim().to_string(),
    );
    let (args, set_args) = server_field(
        &id,
        |s| SharedString::from(s.args.join(" ")),
        |s, v| s.args = v.split_whitespace().map(str::to_string).collect(),
    );
    let status_id = id.clone();

    SettingGroup::new()
        .title(title_case(&id))
        .item(SettingItem::render(move |_, _, cx| {
            match config(cx).language_servers.get(&status_id) {
                Some(server) => server_status(server, cx),
                None => div().into_any_element(),
            }
        }))
        .item(SettingItem::new(
            "Enabled",
            SettingField::switch(enabled, set_enabled),
        ))
        .item(SettingItem::new(
            "Command",
            SettingField::input(command, set_command),
        ))
        .item(SettingItem::new("Arguments", SettingField::input(args, set_args)))
}

fn languages_page(cx: &App) -> SettingPage {
    SettingPage::new("Languages")
        .icon(IconName::Cpu)
        .description(
            "Language servers power Go to Definition, completion, hover and diagnostics. \
             Add more under [language_servers] in config.toml.",
        )
        .groups(config(cx).language_servers.keys().cloned().map(language_group))
}

fn insights_page() -> SettingPage {
    SettingPage::new("Insights").icon(IconName::ChartPie).group(
        SettingGroup::new()
            .description(
                "Scores how you steer Claude Code, from the transcripts in ~/.claude/projects. \
                 Everything stays on this machine; transcripts are read, never copied or sent.",
            )
            .item(SettingItem::new(
                "Enabled",
                SettingField::switch(
                    |cx| config(cx).insights_enabled,
                    |value, cx| update_config(cx, |c| c.insights_enabled = value),
                ),
            ))
            .item(SettingItem::render(|_, _, _| {
                h_flex()
                    .child(div().flex_1())
                    .child(
                        Button::new("clear-insights")
                            .danger()
                            .small()
                            .label("Clear all analytics data")
                            .on_click(|_, window, cx| crate::workspace::clear_insights(window, cx)),
                    )
                    .into_any_element()
            })),
    )
}

fn mcp_page() -> SettingPage {
    SettingPage::new("MCP").icon(IconName::Bot).group(
        SettingGroup::new()
            .description("Local server that lets Claude Code read your editor context. Port changes apply after restart.")
            .item(SettingItem::new(
                "Enabled",
                SettingField::switch(|cx| config(cx).mcp_enabled, |value, cx| update_config(cx, |c| c.mcp_enabled = value)),
            ))
            .item(SettingItem::new(
                "Port",
                SettingField::number_input(
                    number(1024., 65535.),
                    |cx| config(cx).mcp_port as f64,
                    |value, cx| update_config(cx, |c| c.mcp_port = value as u16),
                )
                .default_value(7419.),
            ))
            .item(SettingItem::render(|_, _, cx| {
                let running = cx.try_global::<crate::mcp_bridge::McpStatus>().and_then(|s| s.port);
                let theme = cx.theme();
                h_flex()
                    .gap_2()
                    .text_sm()
                    .child(match running {
                        Some(_) => crate::style::signal_dot(theme.green),
                        None => div().size(px(6.)).rounded_full().bg(theme.muted_foreground),
                    })
                    .child(match running {
                        Some(port) => format!("Running on 127.0.0.1:{port}"),
                        None => "Not running".into(),
                    })
                    .child(div().flex_1())
                    .child(
                        Button::new("copy-mcp")
                            .small()
                            .label("Copy Claude command")
                            .on_click(|_, window, cx| crate::mcp_bridge::copy_claude_command(window, cx)),
                    )
                    .child(
                        Button::new("regen-mcp-token")
                            .ghost()
                            .small()
                            .label("Regenerate token")
                            .on_click(|_, window, cx| {
                                let note = match crate::mcp_bridge::regenerate_token() {
                                    Ok(()) => Notification::warning(
                                        "Restart the IDE, then run the copied command again in Claude Code.",
                                    )
                                    .title("Token regenerated"),
                                    Err(err) => Notification::error(format!("{err:#}")),
                                };
                                window.push_notification(note, cx);
                            }),
                    )
            })),
    )
}

fn keymap_page(workspace: WeakEntity<Workspace>) -> SettingPage {
    SettingPage::new("Keymap").icon(IconName::Frame).group(
        SettingGroup::new()
            .description("JetBrains (macOS) keymap. Override any binding in keymap.toml.")
            .item(SettingItem::render(move |_, window, cx| {
                let workspace = workspace.clone();
                let theme = cx.theme();
                let rows = actions::palette_actions().into_iter().map(|(label, action)| {
                    h_flex()
                        .py_1()
                        .border_b_1()
                        .border_color(theme.border.opacity(0.5))
                        .child(div().flex_1().text_sm().child(label))
                        .children(Kbd::binding_for_action(
                            action.as_ref(),
                            Some(WORKSPACE_CONTEXT),
                            window,
                        ))
                });
                v_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        h_flex().child(div().flex_1()).child(
                            Button::new("open-keymap")
                                .small()
                                .label("Edit keymap.toml")
                                .on_click(move |_, window, cx| {
                                    let Some(path) = ensure_keymap_file() else { return };
                                    workspace
                                        .update(cx, |ws, cx| {
                                            ws.open_file(&path.to_string_lossy(), None, window, cx);
                                        })
                                        .ok();
                                }),
                        ),
                    )
                    .children(rows)
            })),
    )
}

/// Creates a commented keymap.toml on first use and returns its path.
fn ensure_keymap_file() -> Option<PathBuf> {
    let path = Config::dir()?.join("keymap.toml");
    if !path.exists() {
        std::fs::create_dir_all(path.parent()?).ok()?;
        std::fs::write(
            &path,
            "# Power IDEA keymap overrides (applied after restart).\n\
             # Action names: see Settings → Keymap or Find Action.\n\
             #\n\
             # [[binding]]\n\
             # keys = \"cmd-p\"\n\
             # action = \"ide::GotoFile\"\n\
             # context = \"Workspace\"\n",
        )
        .ok()?;
    }
    Some(path)
}

fn project_page(workspace: WeakEntity<Workspace>) -> SettingPage {
    let read = |workspace: &WeakEntity<Workspace>, cx: &App| {
        workspace
            .upgrade()
            .map_or_default(|ws| ws.read(cx).project_settings().clone())
    };
    let (w1, w2, w3, w4, w5) = (
        workspace.clone(),
        workspace.clone(),
        workspace.clone(),
        workspace.clone(),
        workspace,
    );
    let root_label = |ws: &WeakEntity<Workspace>, cx: &App| {
        ws.upgrade()
            .map_or_default(|ws| ws.read(cx).root().display().to_string())
    };

    SettingPage::new("Project").icon(IconName::Folder).group(
        SettingGroup::new()
            .description(
                "Stored in .power-idea/settings.toml inside the project, so it can be shared via git.",
            )
            .item(SettingItem::render(move |_, _, cx| {
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(root_label(&w5, cx))
            }))
            .item(
                SettingItem::new(
                    "Tab size",
                    SettingField::number_input(
                        number(0., 16.),
                        move |cx| read(&w1, cx).tab_size.unwrap_or(0) as f64,
                        move |value, cx| {
                            w2.update(cx, |ws, cx| {
                                ws.update_project_settings(
                                    |p| p.tab_size = (value > 0.).then_some(value as usize),
                                    cx,
                                );
                            })
                            .ok();
                        },
                    )
                    .default_value(0.),
                )
                .description("0 uses the IDE setting."),
            )
            .item(
                SettingItem::new(
                    "Excluded folders",
                    SettingField::input(
                        move |cx| read(&w3, cx).exclude.join(", ").into(),
                        move |value, cx| {
                            let exclude: Vec<String> = value
                                .split(',')
                                .map(|p| p.trim().trim_matches('/').to_string())
                                .filter(|p| !p.is_empty())
                                .collect();
                            w4.update(cx, |ws, cx| {
                                ws.update_project_settings(|p| p.exclude = exclude, cx);
                            })
                            .ok();
                        },
                    ),
                )
                .description(
                    "Comma-separated, e.g. tmp, log, public/packs. Hidden from search and Go to File.",
                ),
            ),
    )
}
