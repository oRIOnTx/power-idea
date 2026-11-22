//! Line breakpoints: toggled with ⌘F8, drawn as red marks in the editor gutter,
//! sent to the running debugger whenever they change. The line where the
//! debugger is paused is highlighted.

use std::collections::BTreeSet;
use std::ops::Range;
use std::path::{Path, PathBuf};

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::input::{GutterDecoration, RangeDecoration, RangeDecorationStyle};
use gpui_kit::*;
use serde_json::json;

use super::super::Workspace;
use crate::editor::CenterTab;

/// Byte range of the 1-based `line` in `text` (without its line break).
pub(super) fn line_range(text: &str, line: u32) -> Option<Range<usize>> {
    let mut start = 0;
    for _ in 1..line {
        start += text[start..].find('\n')? + 1;
    }
    let end = text[start..].find('\n').map_or(text.len(), |n| start + n);
    Some(start..end)
}

impl Workspace {
    /// ⌘F8: toggles a breakpoint on the cursor line of the active editor.
    pub(in crate::workspace) fn toggle_breakpoint(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.active() else { return };
        let (path, line) = (tab.path.clone(), tab.state.read(cx).cursor_position().line + 1);
        let lines = self.breakpoints.entry(path.clone()).or_default();
        if !lines.remove(&line) {
            lines.insert(line);
        }
        if lines.is_empty() {
            self.breakpoints.remove(&path);
        }
        self.refresh_breakpoint_marks(&path, cx);
        self.send_breakpoints(&path, cx);
        cx.notify();
    }

    /// Redraws the gutter marks of `path` if it is open.
    pub(in crate::workspace) fn refresh_breakpoint_marks(&mut self, path: &Path, cx: &mut Context<Self>) {
        let lines = self.breakpoints.get(path).cloned().unwrap_or_default();
        let color = cx.theme().danger;
        for tab in &mut self.tabs {
            let CenterTab::Editor(tab) = tab else { continue };
            if tab.path != path {
                continue;
            }
            let text = tab.state.read(cx).value().to_string();
            let marks: Vec<GutterDecoration> = lines
                .iter()
                .filter_map(|&line| line_range(&text, line))
                .map(|range| GutterDecoration::new(range).with_color(color))
                .collect();
            match &tab.breakpoint_marks {
                Some(collection) => collection.set(marks, cx),
                None => {
                    let collection = tab.state.update(cx, |state, cx| {
                        state.set_gutter_decorations_width(px(6.), cx);
                        state.create_gutter_decorations_collection(marks, cx)
                    });
                    tab.breakpoint_marks = Some(collection);
                }
            }
        }
    }

    /// Sends the breakpoints of one file to the running debugger.
    pub(in crate::workspace) fn send_breakpoints(&self, path: &Path, cx: &mut Context<Self>) {
        let Some(adapter) = self.debug.as_ref().and_then(|d| d.adapter.clone()) else {
            return;
        };
        let lines: Vec<u32> = self
            .breakpoints
            .get(path)
            .map_or_default(|l| l.iter().copied().collect());
        let path = path.to_path_buf();
        cx.background_spawn(async move {
            let _ = adapter.request("setBreakpoints", breakpoint_args(&path, &lines));
        })
        .detach();
    }

    /// All breakpoints, as (file, lines), for the start of a session.
    pub(in crate::workspace) fn all_breakpoints(&self) -> Vec<(PathBuf, Vec<u32>)> {
        self.breakpoints
            .iter()
            .map(|(path, lines)| (path.clone(), lines.iter().copied().collect()))
            .collect()
    }

    /// Highlights the paused line (or clears the highlight when `None`).
    pub(in crate::workspace) fn mark_execution_line(
        &mut self,
        at: Option<(&Path, u32)>,
        cx: &mut Context<Self>,
    ) {
        let color = cx.theme().primary.opacity(0.22);
        for tab in &mut self.tabs {
            let CenterTab::Editor(tab) = tab else { continue };
            let marks = match at {
                Some((path, line)) if tab.path == path => {
                    let text = tab.state.read(cx).value().to_string();
                    line_range(&text, line).map_or_default(|range| {
                        vec![
                            RangeDecoration::new(range)
                                .with_style(RangeDecorationStyle::Fill)
                                .with_color(color),
                        ]
                    })
                }
                _ => Vec::new(),
            };
            match &tab.execution_mark {
                Some(collection) => collection.set(marks, cx),
                None if !marks.is_empty() => {
                    let collection = tab.state.update(cx, |state, cx| {
                        state.create_range_decorations_collection(marks, cx)
                    });
                    tab.execution_mark = Some(collection);
                }
                None => {}
            }
        }
    }
}

pub(super) fn breakpoint_args(path: &Path, lines: &[u32]) -> serde_json::Value {
    let breakpoints: Vec<serde_json::Value> = lines.iter().map(|line| json!({ "line": line })).collect();
    json!({ "source": { "path": path, "name": path.file_name().map(|n| n.to_string_lossy()) }, "breakpoints": breakpoints })
}

/// Breakpoint lines per file.
pub type Breakpoints = std::collections::BTreeMap<PathBuf, BTreeSet<u32>>;

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{breakpoint_args, line_range};

    #[test]
    fn finds_line_ranges() {
        let text = "a = 1\nbb = 2\n\nlast";
        assert_eq!(line_range(text, 1), Some(0..5));
        assert_eq!(line_range(text, 2), Some(6..12));
        assert_eq!(line_range(text, 3), Some(13..13));
        assert_eq!(line_range(text, 4), Some(14..18));
        assert_eq!(line_range(text, 5), None);
    }

    #[test]
    fn builds_set_breakpoints_arguments() {
        let args = breakpoint_args(Path::new("/p/app.rb"), &[2, 9]);
        assert_eq!(args["source"]["path"], "/p/app.rb");
        assert_eq!(args["breakpoints"][1]["line"], 9);
        assert_eq!(
            breakpoint_args(Path::new("/p/app.rb"), &[])["breakpoints"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
    }
}
