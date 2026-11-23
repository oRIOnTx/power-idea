//! The Settings → Claude Code page: how the IDE starts `claude` and what it shows while a
//! session runs. Rules, skills, model and permissions stay in Claude Code's own files.

use claude_integration::ClaudeSettings;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::setting::{
    NumberFieldOptions, SettingField, SettingGroup, SettingItem, SettingPage,
};
use gpui_kit::component::{ActiveTheme as _, IconName, WindowExt as _, h_flex, v_flex};
use gpui_kit::*;

use super::{Binary, state, update_settings};
use crate::style::dialog_button;

fn settings(cx: &App) -> &ClaudeSettings {
    &state(cx).settings
}

/// A switch bound to one boolean of the settings.
fn switch_item(
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

pub fn page() -> SettingPage {
    SettingPage::new("Claude Code")
        .icon(IconName::Bot)
        .description(
            "How the IDE starts claude and what it shows while a session runs. Rules, skills, model and \
             permissions live in Claude Code's own files (CLAUDE.md, ~/.claude). These switches are saved \
             in ~/Library/Application Support/power-idea/claude/settings.toml.",
        )
        .group(general_group())
        .group(gauges_group())
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
    let errors = state.settings_error.iter().cloned();
    let notes = migration_notes(&state.removed_keys, &state.cleaned_up);
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
        .children(errors.map(|error| {
            div()
                .text_sm()
                .text_color(theme.danger)
                .child(format!("⚠ {error}"))
        }))
        .children(
            notes
                .into_iter()
                .map(|note| div().text_sm().text_color(theme.muted_foreground).child(note)),
        )
        .into_any_element()
}

/// What the move to Claude Code's own files changed, shown until the user saves the settings
/// (old keys) or restarts (cleanup).
fn migration_notes(removed_keys: &[String], cleaned_up: &[String]) -> Vec<String> {
    let mut notes = Vec::new();
    if !removed_keys.is_empty() {
        notes.push(format!(
            "Ignored settings of removed features: {}. Claude Code handles these itself; they are dropped \
             from settings.toml the next time you change a setting here.",
            removed_keys.join(", ")
        ));
    }
    if !cleaned_up.is_empty() {
        notes.push(super::cleanup_notice(cleaned_up));
    }
    notes
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

#[cfg(test)]
mod tests {
    use super::migration_notes;

    #[test]
    fn migration_notes_name_what_changed() {
        assert!(migration_notes(&[], &[]).is_empty());
        let notes = migration_notes(
            &["rules".into(), "general.model".into()],
            &["~/.claude/skills/ide-x".into()],
        );
        assert_eq!(notes.len(), 2);
        assert!(notes[0].contains("rules, general.model"));
        assert!(notes[1].contains("~/.claude/skills/ide-x"));
    }
}
