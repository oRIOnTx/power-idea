//! The Settings → Claude Code page: one card per feature, each with its own switch.

use claude_integration::settings::{EFFORT_LEVELS, PERMISSION_MODES, RunOn};
use claude_integration::{ClaudeSettings, Mode};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::setting::{
    NumberFieldOptions, SettingField, SettingGroup, SettingItem, SettingPage,
};
use gpui_kit::component::{ActiveTheme as _, IconName, WindowExt as _, h_flex, v_flex};
use gpui_kit::*;

use super::{Binary, rules_view, skills_view, state, update_settings};
use crate::style::dialog_button;
use crate::workspace::Workspace;

fn settings(cx: &App) -> &ClaudeSettings {
    &state(cx).settings
}

/// A switch bound to one boolean of the settings.
pub(super) fn switch_item(
    title: &'static str,
    get: fn(&ClaudeSettings) -> bool,
    set: fn(&mut ClaudeSettings, bool),
) -> SettingItem {
    SettingItem::new(
        title,
        SettingField::switch(
            move |cx| get(settings(cx)),
            move |value, cx| update_settings(cx, |s| set(s, value)),
        ),
    )
}

pub(super) fn mode_item(get: fn(&ClaudeSettings) -> Mode, set: fn(&mut ClaudeSettings, Mode)) -> SettingItem {
    SettingItem::new(
        "Mode",
        SettingField::dropdown(
            vec![
                ("inject".into(), "Inject when the IDE starts claude".into()),
                (
                    "install".into(),
                    "Install into ~/.claude (also outside the IDE)".into(),
                ),
            ],
            move |cx| match get(settings(cx)) {
                Mode::Inject => "inject".into(),
                Mode::Install => "install".into(),
            },
            move |value, cx| {
                let mode = if value.as_ref() == "install" {
                    Mode::Install
                } else {
                    Mode::Inject
                };
                update_settings(cx, |s| set(s, mode));
            },
        ),
    )
    .description("Inject changes nothing outside the IDE; Install is reversible with Uninstall.")
}

fn choice(
    title: &'static str,
    values: &[&str],
    get: fn(&ClaudeSettings) -> &str,
    set: fn(&mut ClaudeSettings, String),
) -> SettingItem {
    let options = std::iter::once(("".into(), "Claude Code default".into()))
        .chain(values.iter().map(|v| ((*v).into(), (*v).into())))
        .collect();
    SettingItem::new(
        title,
        SettingField::dropdown(
            options,
            move |cx| get(settings(cx)).to_string().into(),
            move |value, cx| update_settings(cx, |s| set(s, value.to_string())),
        ),
    )
}

fn number(
    title: &'static str,
    min: f64,
    max: f64,
    get: fn(&ClaudeSettings) -> f64,
    set: fn(&mut ClaudeSettings, f64),
) -> SettingItem {
    SettingItem::new(
        title,
        SettingField::number_input(
            NumberFieldOptions { min, max, step: 1.0 },
            move |cx| get(settings(cx)),
            move |value, cx| update_settings(cx, |s| set(s, value)),
        ),
    )
}

pub fn page(workspace: WeakEntity<Workspace>) -> SettingPage {
    SettingPage::new("Claude Code")
        .icon(IconName::Bot)
        .description(
            "How the IDE works with Claude Code. Everything is off by default and each feature has its own \
             switch; files live in ~/Library/Application Support/power-idea/claude and can be edited by hand \
             or by an agent (the IDE reloads them).",
        )
        .group(general_group())
        .group(rules_view::group(workspace.clone()))
        .group(skills_group(workspace.clone()))
        .group(quality_group(workspace.clone()))
        .group(gauges_group())
        .group(terminal_group())
        .group(privacy_group())
}

fn general_group() -> SettingGroup {
    SettingGroup::new()
        .title("General")
        .description("The whole section. Off means the IDE starts claude exactly as a terminal would.")
        .item(switch_item(
            "Enable Claude Code integration",
            |s| s.enabled,
            |s, v| s.enabled = v,
        ))
        .item(SettingItem::render(|_, _, cx| status_row(cx)))
        .item(
            SettingItem::new(
                "Path to claude",
                SettingField::input(
                    |cx| settings(cx).general.claude_path.clone().into(),
                    |value, cx| {
                        update_settings(cx, |s| s.general.claude_path = value.trim().to_string());
                        super::detect_binary(cx);
                    },
                ),
            )
            .description("Empty: found on PATH (also ~/.claude/local, ~/.local/bin, Homebrew)."),
        )
        .item(
            SettingItem::new(
                "Model",
                SettingField::input(
                    |cx| settings(cx).general.model.clone().into(),
                    |value, cx| update_settings(cx, |s| s.general.model = value.trim().to_string()),
                ),
            )
            .description("--model: an alias (opus, sonnet, fable) or a full model name. Empty: default."),
        )
        .item(choice(
            "Permission mode",
            PERMISSION_MODES,
            |s| &s.general.permission_mode,
            |s, v| s.general.permission_mode = v,
        ))
        .item(choice(
            "Effort",
            EFFORT_LEVELS,
            |s| &s.general.effort,
            |s, v| s.general.effort = v,
        ))
        .item(
            switch_item(
                "Give Claude the IDE's MCP server",
                |s| s.general.ide_mcp,
                |s, v| s.general.ide_mcp = v,
            )
            .description("--mcp-config with the local MCP server: open files, diagnostics, git state."),
        )
}

/// The detected binary, settings errors and the folder button.
fn status_row(cx: &App) -> AnyElement {
    let state = state(cx);
    let theme = cx.theme();
    let (dot, text): (Div, String) = match &state.binary {
        Binary::Detecting => (
            crate::style::signal_dot(theme.muted_foreground),
            "Looking for claude…".into(),
        ),
        Binary::Found { path, version } => (
            crate::style::signal_dot(theme.green),
            format!("{version} — {}", path.display()),
        ),
        Binary::Missing(reason) => (
            crate::style::signal_dot(theme.danger),
            format!("{reason}. Install: npm install -g @anthropic-ai/claude-code (see docs.claude.com)"),
        ),
    };
    let errors: Vec<String> = state
        .settings_error
        .iter()
        .cloned()
        .chain(
            state
                .rule_errors
                .iter()
                .map(|e| format!("{}: {}", e.path.display(), e.message)),
        )
        .chain(
            state
                .skill_errors
                .iter()
                .map(|e| format!("{}: {}", e.dir.display(), e.message)),
        )
        .collect();
    v_flex()
        .gap_2()
        .child(
            h_flex()
                .gap_2()
                .text_sm()
                .child(dot)
                .child(div().flex_1().min_w_0().child(text))
                .child(
                    dialog_button("claude-detect")
                        .label("Detect again")
                        .on_click(|_, _, cx| super::detect_binary(cx)),
                )
                .child(
                    dialog_button("claude-open-folder")
                        .label("Open folder")
                        .on_click(|_, window, cx| open_folder(window, cx)),
                ),
        )
        .children(errors.into_iter().map(|error| {
            div()
                .text_sm()
                .text_color(theme.danger)
                .child(format!("⚠ {error}"))
        }))
        .into_any_element()
}

fn open_folder(window: &mut Window, cx: &mut App) {
    let root = state(cx).dir.root.clone();
    if let Err(err) = std::process::Command::new("open").arg(&root).spawn() {
        window.push_notification(
            Notification::error(format!("Cannot open {}: {err}", root.display())),
            cx,
        );
    }
}

fn skills_group(workspace: WeakEntity<Workspace>) -> SettingGroup {
    SettingGroup::new()
        .title("Skills library")
        .description(
            "SKILL.md folders in claude/skills. Enabled skills reach Claude as the plugin power-idea \
             (/power-idea:<name>) or, in Install mode, as ~/.claude/skills/ide-<name>.",
        )
        .item(switch_item(
            "Enabled",
            |s| s.skills.enabled,
            |s, v| s.skills.enabled = v,
        ))
        .item(mode_item(|s| s.skills.mode, |s, m| s.skills.mode = m))
        .item(SettingItem::render(move |_, _, cx| {
            skills_view::skills_list(&workspace, cx)
        }))
}

fn quality_group(workspace: WeakEntity<Workspace>) -> SettingGroup {
    SettingGroup::new()
        .title("Code quality")
        .description(
            "Runs the project's linters (clippy, eslint, rubocop, …) and scores the result: \
             score = 100 − error weight × errors − warning weight × warnings, clamped to 0…100.",
        )
        .item(switch_item(
            "Track code quality",
            |s| s.quality.enabled,
            |s, v| s.quality.enabled = v,
        ))
        .item(SettingItem::new(
            "Run",
            SettingField::dropdown(
                vec![
                    ("hook".into(), "After Claude edits a file".into()),
                    ("save".into(), "When a file is saved".into()),
                    ("manual".into(), "Only from the Run button".into()),
                ],
                |cx| match settings(cx).quality.run_on {
                    RunOn::Hook => "hook".into(),
                    RunOn::Save => "save".into(),
                    RunOn::Manual => "manual".into(),
                },
                |value, cx| {
                    let run_on = match value.as_ref() {
                        "save" => RunOn::Save,
                        "manual" => RunOn::Manual,
                        _ => RunOn::Hook,
                    };
                    update_settings(cx, |s| s.quality.run_on = run_on);
                },
            ),
        ))
        .item(number(
            "Error weight",
            0.,
            100.,
            |s| s.quality.error_weight,
            |s, v| s.quality.error_weight = v,
        ))
        .item(number(
            "Warning weight",
            0.,
            100.,
            |s| s.quality.warning_weight,
            |s, v| s.quality.warning_weight = v,
        ))
        .item(SettingItem::render(move |_, _, cx| {
            let root = workspace.upgrade().map(|ws| ws.read(cx).root().clone());
            super::quality::details(root, cx)
        }))
}

fn gauges_group() -> SettingGroup {
    SettingGroup::new()
        .title("Live status")
        .description(
            "Gauges fed by Claude Code hooks and its status line while a session started by the IDE runs. \
             Values are always shown as text as well as color.",
        )
        .item(switch_item(
            "Show live gauges",
            |s| s.gauges.enabled,
            |s, v| s.gauges.enabled = v,
        ))
        .item(switch_item(
            "Context window usage",
            |s| s.gauges.context,
            |s, v| s.gauges.context = v,
        ))
        .item(switch_item(
            "Tokens and cost",
            |s| s.gauges.cost,
            |s, v| s.gauges.cost = v,
        ))
        .item(switch_item(
            "Current activity",
            |s| s.gauges.activity,
            |s, v| s.gauges.activity = v,
        ))
        .item(switch_item(
            "Files changed",
            |s| s.gauges.files,
            |s, v| s.gauges.files = v,
        ))
        .item(switch_item(
            "Code quality score",
            |s| s.gauges.quality,
            |s, v| s.gauges.quality = v,
        ))
        .item(switch_item(
            "In the status bar",
            |s| s.gauges.in_status_bar,
            |s, v| s.gauges.in_status_bar = v,
        ))
        .item(number(
            "Context amber at %",
            1.,
            99.,
            |s| f64::from(s.gauges.context_amber),
            |s, v| {
                s.gauges.context_amber = v as u8;
            },
        ))
        .item(number(
            "Context red at %",
            2.,
            100.,
            |s| f64::from(s.gauges.context_red),
            |s, v| {
                s.gauges.context_red = v as u8;
            },
        ))
}

fn terminal_group() -> SettingGroup {
    SettingGroup::new()
        .title("Terminal")
        .description("A pinned Claude tab first in the terminal panel, started in the project root with the injected settings.")
        .item(switch_item("Claude tab", |s| s.terminal.claude_tab, |s, v| s.terminal.claude_tab = v))
        .item(switch_item("Start automatically", |s| s.terminal.auto_start, |s, v| s.terminal.auto_start = v))
        .item(
            switch_item("Continue the last session", |s| s.terminal.continue_last, |s, v| s.terminal.continue_last = v)
                .description("--continue instead of a new session."),
        )
}

fn privacy_group() -> SettingGroup {
    SettingGroup::new()
        .title("Privacy")
        .description(
            "The IDE reads only local files: its claude folder, ~/.claude settings and the session transcripts \
             in ~/.claude/projects. Nothing is sent anywhere; Claude Code itself talks to Anthropic as usual.",
        )
        .item(SettingItem::render(|_, _, _| {
            h_flex()
                .child(div().flex_1())
                .child(
                    dialog_button("claude-clear-cache")
                        .label("Clear IDE caches")
                        .on_click(|_, window, cx| clear_cache(window, cx)),
                )
                .into_any_element()
        }))
}

fn clear_cache(window: &mut Window, cx: &mut App) {
    let Some(cache) = super::cache_root() else { return };
    let result = if cache.exists() {
        std::fs::remove_dir_all(&cache)
    } else {
        Ok(())
    };
    let note = match result {
        Ok(()) => {
            Notification::success("Generated Claude files were removed; they are rebuilt on the next launch.")
        }
        Err(err) => Notification::error(format!("Cannot clear {}: {err}", cache.display())),
    };
    window.push_notification(note, cx);
}
