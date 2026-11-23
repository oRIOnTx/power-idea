//! The skills card: a switch per skill in claude/skills, which ones the project's Claude
//! session has, editing, and adding skills from a folder or a Git URL.

use claude_integration::skills::{self, ImportReport};
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::gauges::ClaudeGauges;
use super::rules_view::open_file;
use super::{reload, state};
use crate::style::dialog_button;
use crate::workspace::Workspace;

/// Rows of the skills card plus the add buttons.
pub fn skills_list(workspace: &WeakEntity<Workspace>, cx: &App) -> AnyElement {
    let state = state(cx);
    let theme = cx.theme();
    let disabled = state.settings.skills.disabled.clone();
    let active: Vec<String> = workspace
        .upgrade()
        .map(|ws| ws.read(cx).root().to_string_lossy().into_owned())
        .and_then(|tag| cx.try_global::<ClaudeGauges>()?.active_skills.get(&tag).cloned())
        .unwrap_or_default();
    let rows = state.skills.iter().enumerate().map(|(ix, skill)| {
        let name = skill.name.clone();
        let file = skill.dir.join(skills::SKILL_FILE);
        let edit_ws = workspace.clone();
        let is_active = active.contains(&skill.name);
        h_flex()
            .gap_2()
            .px_2()
            .py_1p5()
            .when(ix > 0, |el| el.border_t_1().border_color(theme.border))
            .child(
                Switch::new(("claude-skill-on", ix))
                    .small()
                    .checked(!disabled.contains(&skill.name))
                    .on_click(move |checked, _, cx| {
                        let (name, checked) = (name.clone(), *checked);
                        super::update_settings(cx, |s| {
                            s.skills.disabled.retain(|d| *d != name);
                            if !checked {
                                s.skills.disabled.push(name);
                                s.skills.disabled.sort();
                            }
                        });
                    }),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(div().text_sm().child(skill.name.clone()))
                            .when(skill.builtin, |el| {
                                el.child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .child("built-in"),
                                )
                            })
                            .when(is_active, |el| {
                                el.child(
                                    div()
                                        .px_1p5()
                                        .rounded(px(4.))
                                        .text_xs()
                                        .bg(theme.green.opacity(0.15))
                                        .text_color(theme.green)
                                        .child("active in session"),
                                )
                            }),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(skill.description.clone()),
                    ),
            )
            .child(
                component::button::Button::new(("skill-edit", ix))
                    .ghost()
                    .xsmall()
                    .label("Edit")
                    .on_click(move |_, window, cx| open_file(&edit_ws, &file, window, cx)),
            )
    });
    v_flex()
        .gap_2()
        .child(
            v_flex()
                .rounded(theme.radius)
                .border_1()
                .border_color(theme.border)
                .children(rows)
                .when(state.skills.is_empty(), |el| {
                    el.child(
                        div()
                            .p_3()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child("No skills. Add SKILL.md folders to claude/skills."),
                    )
                }),
        )
        .child(
            h_flex()
                .gap_2()
                .child(
                    dialog_button("skill-add-folder")
                        .label("Add from folder…")
                        .on_click(|_, _, cx| add_from_folder(cx)),
                )
                .child(
                    dialog_button("skill-add-git")
                        .label("Add from Git URL…")
                        .on_click(|_, window, cx| ask_git_url(window, cx)),
                ),
        )
        .into_any_element()
}

fn add_from_folder(cx: &mut App) {
    let picked = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some("Add Skills".into()),
    });
    let library = state(cx).dir.skills();
    cx.spawn(async move |cx| {
        let Ok(Ok(Some(paths))) = picked.await else { return };
        let Some(source) = paths.into_iter().next() else {
            return;
        };
        let result = cx
            .background_spawn(async move { skills::import(&skills::find_skill_dirs(&source, 4), &library) })
            .await;
        cx.update(|cx| finish(result, cx));
    })
    .detach();
}

fn ask_git_url(window: &mut Window, cx: &mut App) {
    let input = cx.new(|cx| InputState::new(window, cx).placeholder("https://github.com/owner/skills.git"));
    let focused = std::cell::Cell::new(false);
    window.open_dialog(cx, move |dialog, window, cx| {
        if !focused.replace(true) {
            input.update(cx, |i, cx| i.focus(window, cx));
        }
        let url_input = input.clone();
        dialog
            .w(px(520.))
            .title("Add skills from a Git repository")
            .child(v_flex().gap_2().child(Input::new(&input)).child(
                div().text_sm().text_color(cx.theme().muted_foreground).child(
                    "The repository is cloned once (depth 1); every folder with a SKILL.md is copied into \
                         claude/skills. Only https:// and git@ URLs.",
                ),
            ))
            .footer(
                h_flex()
                    .w_full()
                    .gap_2()
                    .justify_end()
                    .child(
                        dialog_button("git-cancel")
                            .ghost()
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        dialog_button("git-add")
                            .primary()
                            .label("Add")
                            .on_click(move |_, window, cx| {
                                let url = url_input.read(cx).value().trim().to_string();
                                if let Err(err) = skills::validate_git_url(&url) {
                                    window.push_notification(Notification::error(format!("{err:#}")), cx);
                                    return;
                                }
                                window.close_dialog(cx);
                                add_from_git(url, cx);
                            }),
                    ),
            )
    });
}

fn add_from_git(url: String, cx: &mut App) {
    let library = state(cx).dir.skills();
    let Some(checkout) = super::cache_root().map(|c| c.join("git-import")) else {
        return;
    };
    cx.spawn(async move |cx| {
        let result = cx
            .background_spawn(async move {
                if checkout.exists() {
                    std::fs::remove_dir_all(&checkout)?;
                }
                let output = std::process::Command::new("git")
                    .args(["clone", "--depth", "1", "--quiet", "--"])
                    .arg(&url)
                    .arg(&checkout)
                    .env("GIT_TERMINAL_PROMPT", "0")
                    .output()?;
                if !output.status.success() {
                    anyhow::bail!(
                        "git clone failed: {}",
                        String::from_utf8_lossy(&output.stderr).trim()
                    );
                }
                let report = skills::import(&skills::find_skill_dirs(&checkout, 4), &library);
                let _ = std::fs::remove_dir_all(&checkout);
                report
            })
            .await;
        cx.update(|cx| finish(result, cx));
    })
    .detach();
}

fn finish(result: Result<ImportReport>, cx: &mut App) {
    let note = match result {
        Ok(report) if report.added.is_empty() && report.skipped.is_empty() => {
            Notification::info("No SKILL.md folders were found.")
        }
        Ok(report) => {
            let mut lines: Vec<String> = report.added.iter().map(|n| format!("Added {n}")).collect();
            lines.extend(
                report
                    .skipped
                    .iter()
                    .map(|(n, why)| format!("Skipped {n}: {why}")),
            );
            Notification::success(lines.join("\n")).title("Skills")
        }
        Err(err) => Notification::error(format!("{err:#}")),
    };
    if let Some(window) = cx.active_window() {
        window
            .update(cx, |_, window, cx| window.push_notification(note, cx))
            .ok();
    }
    reload(cx);
}

/// The skills a project's session was started with (set when the Claude tab starts).
pub fn remember_active(project_root: &std::path::Path, names: Vec<String>, cx: &mut App) {
    if cx.has_global::<ClaudeGauges>() {
        let tag = project_root.to_string_lossy().into_owned();
        cx.global_mut::<ClaudeGauges>().active_skills.insert(tag, names);
    }
}
