//! The buttons under the rules list: templates, folder import/export, preview, export to
//! project and the global install / uninstall.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use claude_integration::installer::{Manifest, export_to_project};
use claude_integration::rules::{self, TEMPLATES};
use claude_integration::{Mode, Rule, resolver};
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{WindowExt as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::rules_view::{facts, open_file};
use super::{reload, state};
use crate::style::dialog_button;
use crate::workspace::Workspace;

pub(super) fn actions(workspace: &WeakEntity<Workspace>, cx: &App) -> AnyElement {
    let state = state(cx);
    let install_mode =
        state.settings.rules.mode == Mode::Install || state.settings.skills.mode == Mode::Install;
    let installed = state.manifest != Manifest::default();
    let (template_ws, preview_ws, export_ws) = (workspace.clone(), workspace.clone(), workspace.clone());
    h_flex()
        .flex_wrap()
        .gap_2()
        .child(
            dialog_button("rule-new")
                .label("New from template")
                .dropdown_caret(true)
                .dropdown_menu(move |mut menu, _, _| {
                    for (title, text) in TEMPLATES.iter().copied().chain([("Blank rule", BLANK)]) {
                        let workspace = template_ws.clone();
                        menu = menu.item(PopupMenuItem::new(title).on_click(move |_, window, cx| {
                            create_rule(title, text, &workspace, window, cx);
                        }));
                    }
                    menu
                }),
        )
        .child(
            dialog_button("rule-import")
                .label("Import folder…")
                .on_click(|_, _, cx| import_folder(cx)),
        )
        .child(
            dialog_button("rule-export")
                .label("Export folder…")
                .on_click(|_, _, cx| export_folder(cx)),
        )
        .child(
            dialog_button("rule-preview")
                .label("Preview effective context")
                .on_click(move |_, window, cx| {
                    if let Some(facts) = facts(&preview_ws, cx) {
                        super::open_preview(&facts, window, cx);
                    }
                }),
        )
        .child(
            dialog_button("rule-export-project")
                .label("Export to project")
                .on_click(move |_, window, cx| export_project(&export_ws, window, cx)),
        )
        .when(install_mode, |el| {
            el.child(
                dialog_button("claude-install")
                    .primary()
                    .label(if installed {
                        "Update ~/.claude"
                    } else {
                        "Install into ~/.claude"
                    })
                    .on_click(|_, window, cx| install(false, window, cx)),
            )
        })
        .when(installed, |el| {
            el.child(
                dialog_button("claude-uninstall")
                    .label("Uninstall")
                    .on_click(|_, window, cx| install(true, window, cx)),
            )
        })
        .into_any_element()
}

const BLANK: &str = "---\nname: \"New rule\"\nenabled: true\napplies_to: \"all\"\npriority: 0\ntags: []\n---\n\n\
                     Write the instructions for Claude here.\n";

fn create_rule(
    title: &str,
    text: &str,
    workspace: &WeakEntity<Workspace>,
    window: &mut Window,
    cx: &mut App,
) {
    let dir = state(cx).dir.rules();
    let path = rules::new_rule_path(&dir, title);
    if let Err(err) = fs::create_dir_all(&dir).and_then(|()| fs::write(&path, text)) {
        window.push_notification(
            Notification::error(format!("Cannot create {}: {err}", path.display())),
            cx,
        );
        return;
    }
    reload(cx);
    open_file(workspace, &path, window, cx);
}

fn import_folder(cx: &mut App) {
    let picked = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some("Import Rules".into()),
    });
    let target = state(cx).dir.rules();
    cx.spawn(async move |cx| {
        let Ok(Ok(Some(paths))) = picked.await else { return };
        let Some(source) = paths.into_iter().next() else {
            return;
        };
        let result = cx
            .background_spawn(async move { copy_rules(&source, &target, true) })
            .await;
        cx.update(|cx| {
            report(cx, "Imported", result);
            reload(cx);
        });
    })
    .detach();
}

fn export_folder(cx: &mut App) {
    let picked = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some("Export Rules".into()),
    });
    let source = state(cx).dir.rules();
    cx.spawn(async move |cx| {
        let Ok(Ok(Some(paths))) = picked.await else { return };
        let Some(target) = paths.into_iter().next() else {
            return;
        };
        let result = cx
            .background_spawn(async move { copy_rules(&source, &target, false) })
            .await;
        cx.update(|cx| report(cx, "Exported", result));
    })
    .detach();
}

/// Copies valid rule files; on import, names that exist get a new suffix instead of overwriting.
fn copy_rules(from: &Path, to: &Path, rename: bool) -> Result<usize> {
    fs::create_dir_all(to)?;
    let mut copied = 0;
    for entry in fs::read_dir(from)? {
        let path = entry?.path();
        if path.extension().is_none_or(|e| e != "md") {
            continue;
        }
        let text = fs::read_to_string(&path)?;
        let Ok(rule) = Rule::parse(&text, &path) else {
            continue;
        };
        let target = if rename {
            rules::new_rule_path(to, &rule.meta.name)
        } else {
            to.join(path.file_name().unwrap_or_default())
        };
        fs::write(target, text)?;
        copied += 1;
    }
    Ok(copied)
}

fn report(cx: &mut App, verb: &str, result: Result<usize>) {
    let note = match result {
        Ok(count) => Notification::success(format!("{verb} {count} rule file(s).")),
        Err(err) => Notification::error(format!("{err:#}")),
    };
    if let Some(window) = cx.active_window() {
        window
            .update(cx, |_, window, cx| window.push_notification(note, cx))
            .ok();
    }
}

fn export_project(workspace: &WeakEntity<Workspace>, window: &mut Window, cx: &mut App) {
    let Some(facts) = facts(workspace, cx) else { return };
    let state = state(cx);
    let effective = resolver::resolve(&state.settings, &state.rules, &state.skills, &facts);
    if effective.rules_text.is_empty() {
        let note = Notification::info("No enabled rule applies to this project; nothing to export.");
        window.push_notification(note, cx);
        return;
    }
    let note = match export_to_project(&facts.root, &effective.rules_text) {
        Ok(true) => Notification::success("Rules written to the managed block in CLAUDE.md."),
        Ok(false) => Notification::info("CLAUDE.md already has these rules."),
        Err(err) => Notification::error(format!("{err:#}")),
    };
    window.push_notification(note.title("Export to project"), cx);
}

/// Install / update (or uninstall) the global copy, in the background.
fn install(uninstall: bool, window: &mut Window, cx: &mut App) {
    let state = state(cx);
    let Some(installer) = super::claude_home()
        .map(|home| claude_integration::installer::Installer::new(home, state.dir.root.clone()))
    else {
        return;
    };
    let settings = &state.settings;
    let rules_text = (settings.enabled && settings.rules.enabled && settings.rules.mode == Mode::Install)
        .then(|| {
            let enabled: Vec<&Rule> = state.rules.iter().filter(|r| r.meta.enabled).collect();
            resolver::rules_text(&enabled, true)
        })
        .filter(|text| !text.is_empty());
    let skill_dirs: Vec<PathBuf> =
        if settings.enabled && settings.skills.enabled && settings.skills.mode == Mode::Install {
            claude_integration::skills::enabled(&state.skills, &settings.skills.disabled)
                .iter()
                .map(|s| s.dir.clone())
                .collect()
        } else {
            Vec::new()
        };
    let window_handle = window.window_handle();
    cx.spawn(async move |cx| {
        let result = cx
            .background_spawn(async move {
                if uninstall {
                    installer.uninstall()
                } else {
                    installer.install(rules_text.as_deref(), &skill_dirs)
                }
            })
            .await;
        cx.update(|cx| {
            let note = match result {
                Ok(report) if report.changed.is_empty() && report.skipped.is_empty() => {
                    Notification::info("~/.claude is already up to date.")
                }
                Ok(report) => Notification::success(
                    report
                        .changed
                        .iter()
                        .chain(&report.skipped)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("\n"),
                )
                .title(if uninstall {
                    "Uninstalled"
                } else {
                    "Installed into ~/.claude"
                }),
                Err(err) => Notification::error(format!("{err:#}")),
            };
            window_handle
                .update(cx, |_, window, cx| window.push_notification(note, cx))
                .ok();
            reload(cx);
        });
    })
    .detach();
}
