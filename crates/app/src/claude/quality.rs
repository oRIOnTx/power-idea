//! Code quality tracking: runs the project's linters in the background (after Claude edits a
//! file, on save, or from the Run button) and feeds the score into the gauges.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use claude_integration::quality::{self, Tool};
use claude_integration::settings::RunOn;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::*;

use super::gauges::ClaudeGauges;
use super::state;

/// Which projects are being measured, which need another run, and the last result per project.
#[derive(Default)]
struct QualityRuns {
    running: HashSet<String>,
    again: HashSet<String>,
    last: HashMap<String, String>,
}

impl Global for QualityRuns {}

fn tracking(cx: &App) -> Option<RunOn> {
    let settings = &state(cx).settings;
    (settings.enabled && settings.quality.enabled).then_some(settings.quality.run_on)
}

/// Claude edited a file in the session of `project_root`.
pub fn on_claude_edit(project_root: &Path, cx: &mut App) {
    if tracking(cx) == Some(RunOn::Hook) {
        run(project_root, cx);
    }
}

/// A file of `project_root` was saved in the IDE.
pub fn on_file_saved(project_root: &Path, cx: &mut App) {
    if tracking(cx) == Some(RunOn::Save) {
        run(project_root, cx);
    }
}

/// Measures `project_root` now; a request during a run schedules one more run after it.
pub fn run(project_root: &Path, cx: &mut App) {
    let tag = project_root.to_string_lossy().into_owned();
    let runs = cx.default_global::<QualityRuns>();
    if !runs.running.insert(tag.clone()) {
        runs.again.insert(tag);
        return;
    }
    let root = project_root.to_path_buf();
    let settings = state(cx).settings.quality.clone();
    cx.spawn(async move |cx| {
        let measured = cx.background_spawn({
            let root = root.clone();
            async move { measure(&root) }
        });
        let (errors, warnings, summary) = measured.await;
        cx.update(|cx| {
            let sample = quality::sample(errors, warnings, &settings);
            if cx.has_global::<ClaudeGauges>() {
                let gauges = cx.global_mut::<ClaudeGauges>();
                gauges
                    .sessions
                    .entry(tag.clone())
                    .or_default()
                    .apply_quality(sample);
            }
            let runs = cx.global_mut::<QualityRuns>();
            runs.running.remove(&tag);
            runs.last
                .insert(tag.clone(), format!("{summary} → score {:.0}", sample.score));
            let again = runs.again.remove(&tag);
            cx.refresh_windows();
            if again {
                run(&root, cx);
            }
        });
    })
    .detach();
}

/// Runs every detected tool; returns error and warning totals and a one-line summary.
fn measure(root: &Path) -> (u32, u32, String) {
    let tools = quality::detect(root);
    if tools.is_empty() {
        return (0, 0, "no linters detected".into());
    }
    let (mut errors, mut warnings) = (0, 0);
    let mut parts = Vec::new();
    for tool in &tools {
        match run_tool(tool, root) {
            Some((e, w)) => {
                errors += e;
                warnings += w;
                parts.push(format!("{} {e}E {w}W", tool.name));
            }
            None => parts.push(format!("{} not available", tool.name)),
        }
    }
    (errors, warnings, parts.join(", "))
}

fn run_tool(tool: &Tool, root: &Path) -> Option<(u32, u32)> {
    let output = Command::new(tool.program)
        .args(tool.args)
        .current_dir(root)
        .env("NO_COLOR", "1")
        .env("CARGO_TERM_COLOR", "never")
        .output()
        .ok()?;
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Some(quality::count(tool.parser, &text))
}

/// The card's details for a project: detected tools, the last result and a Run button.
pub fn details(project_root: Option<PathBuf>, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let Some(root) = project_root else {
        return div().into_any_element();
    };
    let tag = root.to_string_lossy().into_owned();
    let tools: Vec<&str> = quality::detect(&root).iter().map(|t| t.name).collect();
    let runs = cx.try_global::<QualityRuns>();
    let running = runs.is_some_and(|r| r.running.contains(&tag));
    let last = runs.and_then(|r| r.last.get(&tag).cloned());
    let detected = if tools.is_empty() {
        "No linters detected in this project.".to_string()
    } else {
        format!("Detected: {}", tools.join(", "))
    };
    h_flex()
        .gap_2()
        .child(
            v_flex().flex_1().min_w_0().text_sm().child(detected).child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(if running {
                        "Running…".to_string()
                    } else {
                        last.unwrap_or_default()
                    }),
            ),
        )
        .child(
            crate::style::dialog_button("quality-run")
                .label("Run now")
                .on_click(move |_, _, cx| run(&root, cx)),
        )
        .into_any_element()
}
