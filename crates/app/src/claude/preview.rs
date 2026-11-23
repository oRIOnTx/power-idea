//! The launch for a project (shared by the Claude terminal tab) and "Preview effective
//! context": the exact command line and generated files, secrets redacted.

use std::path::PathBuf;

use claude_integration::launcher::{self, LaunchOptions, LaunchPlan, McpEndpoint};
use claude_integration::{ProjectFacts, resolve};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::*;

use super::{Binary, ClaudeState, cache_root};
use crate::mcp_bridge::McpStatus;

/// The launch plan for `facts`'s project and its cache folder.
pub fn plan_for(facts: &ProjectFacts, continue_last: bool, cx: &App) -> Option<LaunchPlan> {
    let state = cx.try_global::<ClaudeState>()?;
    let settings = &state.settings;
    let effective = resolve(settings, &state.rules, &state.skills, facts);
    let program = match &state.binary {
        Binary::Found { path, .. } => path.clone(),
        _ => PathBuf::from("claude"),
    };
    let cache = cache_root()?.join(launcher::cache_key(&facts.root));
    let home = dirs::home_dir().unwrap_or_default();
    let mcp = settings.general.ide_mcp.then(|| mcp_endpoint(cx)).flatten();
    let options = LaunchOptions {
        ide_exe: std::env::current_exe().ok(),
        socket: Some(super::bridge_socket()),
        tag: facts.root.to_string_lossy().into_owned(),
        mcp,
        user_statusline: launcher::user_statusline(&facts.root, &home),
    };
    Some(launcher::plan(
        settings,
        &effective,
        &program,
        &facts.root,
        &cache,
        continue_last,
        &options,
    ))
}

fn mcp_endpoint(cx: &App) -> Option<McpEndpoint> {
    let port = cx.try_global::<McpStatus>()?.port?;
    let token = crate::mcp_bridge::load_or_create_token().ok()?;
    Some(McpEndpoint { port, token })
}

/// Opens a dialog with what `claude` would get for this project.
pub fn open_preview(facts: &ProjectFacts, window: &mut Window, cx: &mut App) {
    let continue_last = cx
        .try_global::<ClaudeState>()
        .is_some_and(|s| s.settings.terminal.continue_last);
    let Some(plan) = plan_for(facts, continue_last, cx) else {
        window.push_notification(Notification::error("No config folder for Claude Code."), cx);
        return;
    };
    let text: SharedString = launcher::preview(&plan).into();
    let title = format!("Effective Claude Code context — {}", facts.name());
    let precedence = "Precedence: the project's CLAUDE.md and .claude settings > these IDE rules.";
    window.open_dialog(cx, move |dialog, _, cx| {
        let theme = cx.theme();
        let copy_text = text.clone();
        dialog
            .w(px(820.))
            .title(title.clone())
            .child(
                v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(precedence),
                    )
                    .child(
                        div()
                            .id("claude-preview")
                            .max_h(px(520.))
                            .overflow_y_scroll()
                            .p_3()
                            .rounded(theme.radius)
                            .border_1()
                            .border_color(theme.border)
                            .bg(theme.background)
                            .font_family(theme.mono_font_family.clone())
                            .text_xs()
                            .whitespace_normal()
                            .child(text.clone()),
                    ),
            )
            .footer(
                h_flex()
                    .w_full()
                    .gap_2()
                    .justify_end()
                    .child(
                        crate::style::dialog_button("preview-copy")
                            .label("Copy")
                            .on_click(move |_, window, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(copy_text.to_string()));
                                window.push_notification(Notification::success("Copied"), cx);
                            }),
                    )
                    .child(
                        crate::style::dialog_button("preview-close")
                            .label("Close")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
            )
    });
}
