//! Code quality tracking: runs the project's linters and feeds the score into the gauges.

use std::path::Path;

use gpui_kit::App;

/// Claude edited a file in the session of `project_root`.
pub fn on_claude_edit(_project_root: &Path, _cx: &mut App) {}
