//! Live gauges of a Claude Code session, folded from hook events and status-line updates
//! (field names as documented, see docs/claude-code-integration.md). Pure: no I/O.

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::Value;

/// What Claude is doing right now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Activity {
    #[default]
    Idle,
    /// Working on a prompt (between tools).
    Thinking,
    /// Running a tool: name and what it works on (file, command, pattern).
    Tool {
        name: String,
        detail: String,
    },
    /// Waiting for the user (a permission prompt or idle notification).
    Waiting(String),
    Ended,
}

impl Activity {
    pub fn label(&self) -> String {
        match self {
            Self::Idle => "Idle".into(),
            Self::Thinking => "Thinking…".into(),
            Self::Tool { name, detail } if detail.is_empty() => name.clone(),
            Self::Tool { name, detail } => {
                let verb = match name.as_str() {
                    "Edit" | "MultiEdit" | "Write" | "NotebookEdit" => "Editing",
                    "Read" => "Reading",
                    "Bash" => "Running",
                    "Grep" | "Glob" => "Searching",
                    other => other,
                };
                format!("{verb} {detail}")
            }
            Self::Waiting(message) => message.clone(),
            Self::Ended => "Session ended".into(),
        }
    }
}

/// Lint counts measured for the session (see `quality`).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct QualitySample {
    pub errors: u32,
    pub warnings: u32,
    pub score: f64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct GaugeState {
    pub session_id: String,
    pub model: String,
    /// `context_window.used_percentage` from the status line.
    pub context_percent: Option<f64>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: Option<f64>,
    pub lines_added: u64,
    pub lines_removed: u64,
    pub activity: Activity,
    /// Files edited by Claude in this session, relative to the project when inside it.
    pub files_changed: BTreeSet<String>,
    /// Quality at the session start and now.
    pub quality_start: Option<QualitySample>,
    pub quality: Option<QualitySample>,
    /// A status-line update was seen (so transcript fallbacks are not needed).
    pub has_status: bool,
}

const EDIT_TOOLS: &[&str] = &["Edit", "MultiEdit", "Write", "NotebookEdit"];

impl GaugeState {
    /// Applies one hook event's JSON. Returns true when the event edited a file.
    pub fn apply_hook(&mut self, event: &Value, project_root: &Path) -> bool {
        let name = event
            .get("hook_event_name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let session = str_field(event, "session_id");
        if name == "SessionStart" {
            let source = str_field(event, "source");
            // A resumed or compacted session keeps its counters; a new or cleared one starts over.
            if !matches!(source.as_str(), "resume" | "compact") || session != self.session_id {
                *self = Self {
                    has_status: self.has_status,
                    ..Self::default()
                };
            }
            self.session_id = session;
            if let Some(model) = event.get("model").and_then(Value::as_str) {
                self.model = model.to_string();
            }
            self.activity = Activity::Idle;
            return false;
        }
        if !session.is_empty() {
            self.session_id = session;
        }
        match name {
            "UserPromptSubmit" => self.activity = Activity::Thinking,
            "PreToolUse" => {
                let tool = str_field(event, "tool_name");
                let detail = tool_detail(event.get("tool_input"), project_root);
                self.activity = Activity::Tool { name: tool, detail };
            }
            "PostToolUse" => {
                self.activity = Activity::Thinking;
                let tool = str_field(event, "tool_name");
                if EDIT_TOOLS.contains(&tool.as_str())
                    && let Some(path) = file_path(event.get("tool_input"))
                {
                    self.files_changed.insert(relative(&path, project_root));
                    return true;
                }
            }
            "Notification" => {
                self.activity = Activity::Waiting(str_field(event, "message"));
            }
            "Stop" => self.activity = Activity::Idle,
            "SessionEnd" => self.activity = Activity::Ended,
            _ => {}
        }
        false
    }

    /// Applies one status-line JSON.
    pub fn apply_status(&mut self, status: &Value) {
        self.has_status = true;
        let number = |pointer: &str| status.pointer(pointer).and_then(Value::as_f64);
        let count = |pointer: &str| status.pointer(pointer).and_then(Value::as_u64);
        if let Some(session) = status.get("session_id").and_then(Value::as_str) {
            self.session_id = session.to_string();
        }
        if let Some(model) = status.pointer("/model/display_name").and_then(Value::as_str) {
            self.model = model.to_string();
        }
        if let Some(percent) = number("/context_window/used_percentage") {
            self.context_percent = Some(percent.clamp(0.0, 100.0));
        }
        if let Some(tokens) = count("/context_window/total_input_tokens") {
            self.input_tokens = tokens;
        }
        if let Some(tokens) = count("/context_window/total_output_tokens") {
            self.output_tokens = tokens;
        }
        if let Some(cost) = number("/cost/total_cost_usd") {
            self.cost_usd = Some(cost);
        }
        if let Some(lines) = count("/cost/total_lines_added") {
            self.lines_added = lines;
        }
        if let Some(lines) = count("/cost/total_lines_removed") {
            self.lines_removed = lines;
        }
    }

    /// Records a quality measurement; the first one of the session is the baseline.
    pub fn apply_quality(&mut self, sample: QualitySample) {
        if self.quality_start.is_none() {
            self.quality_start = Some(sample);
        }
        self.quality = Some(sample);
    }

    /// Change in errors and warnings since the session started (negative = better).
    pub fn quality_delta(&self) -> Option<(i64, i64)> {
        let (start, now) = (self.quality_start?, self.quality?);
        Some((
            i64::from(now.errors) - i64::from(start.errors),
            i64::from(now.warnings) - i64::from(start.warnings),
        ))
    }

    pub fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }
}

fn str_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn file_path(input: Option<&Value>) -> Option<String> {
    let input = input?;
    ["file_path", "notebook_path", "path"]
        .iter()
        .find_map(|key| input.get(*key).and_then(Value::as_str))
        .map(str::to_string)
}

/// A short description of what a tool works on.
fn tool_detail(input: Option<&Value>, project_root: &Path) -> String {
    let Some(input) = input else { return String::new() };
    if let Some(path) = file_path(Some(input)) {
        return relative(&path, project_root);
    }
    for key in ["command", "pattern", "url", "query", "description"] {
        if let Some(text) = input.get(key).and_then(Value::as_str) {
            let line = text.lines().next().unwrap_or_default();
            return truncate(line, 60);
        }
    }
    String::new()
}

fn relative(path: &str, root: &Path) -> String {
    Path::new(path)
        .strip_prefix(root)
        .map_or_else(|_| path.to_string(), |rel| rel.to_string_lossy().into_owned())
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{cut}…")
}

/// Color band of a percentage against the amber / red thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Band {
    Good,
    Warn,
    Bad,
}

pub fn band(percent: f64, amber: u8, red: u8) -> Band {
    if percent >= f64::from(red) {
        Band::Bad
    } else if percent >= f64::from(amber) {
        Band::Warn
    } else {
        Band::Good
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn root() -> &'static Path {
        Path::new("/work/app")
    }

    #[test]
    fn follows_hook_events() {
        let mut state = GaugeState::default();
        state.apply_hook(&json!({ "hook_event_name": "SessionStart", "session_id": "s1", "source": "startup", "model": "opus" }), root());
        assert_eq!((state.session_id.as_str(), state.model.as_str()), ("s1", "opus"));
        state.apply_hook(
            &json!({ "hook_event_name": "UserPromptSubmit", "prompt": "fix it" }),
            root(),
        );
        assert_eq!(state.activity, Activity::Thinking);
        state.apply_hook(
            &json!({ "hook_event_name": "PreToolUse", "tool_name": "Edit", "tool_input": { "file_path": "/work/app/src/main.rs" } }),
            root(),
        );
        assert_eq!(state.activity.label(), "Editing src/main.rs");
        let edited = state.apply_hook(
            &json!({ "hook_event_name": "PostToolUse", "tool_name": "Edit", "tool_input": { "file_path": "/work/app/src/main.rs" } }),
            root(),
        );
        assert!(edited);
        state.apply_hook(
            &json!({ "hook_event_name": "PreToolUse", "tool_name": "Bash", "tool_input": { "command": "cargo test\nmore" } }),
            root(),
        );
        assert_eq!(state.activity.label(), "Running cargo test");
        state.apply_hook(
            &json!({ "hook_event_name": "PostToolUse", "tool_name": "Bash", "tool_input": {} }),
            root(),
        );
        state.apply_hook(
            &json!({ "hook_event_name": "Notification", "message": "Claude needs your permission" }),
            root(),
        );
        assert_eq!(state.activity.label(), "Claude needs your permission");
        state.apply_hook(&json!({ "hook_event_name": "Stop" }), root());
        assert_eq!(state.activity, Activity::Idle);
        assert_eq!(state.files_changed.iter().collect::<Vec<_>>(), ["src/main.rs"]);

        // Resume keeps the counters; a new session starts over.
        state.apply_hook(
            &json!({ "hook_event_name": "SessionStart", "session_id": "s1", "source": "resume" }),
            root(),
        );
        assert_eq!(state.files_changed.len(), 1);
        state.apply_hook(
            &json!({ "hook_event_name": "SessionStart", "session_id": "s2", "source": "startup" }),
            root(),
        );
        assert!(state.files_changed.is_empty());
        state.apply_hook(
            &json!({ "hook_event_name": "SessionEnd", "reason": "other" }),
            root(),
        );
        assert_eq!(state.activity, Activity::Ended);
    }

    #[test]
    fn reads_the_status_line() {
        let mut state = GaugeState::default();
        state.apply_status(&json!({
            "session_id": "s1",
            "model": { "id": "claude-opus", "display_name": "Opus" },
            "context_window": { "used_percentage": 42.5, "total_input_tokens": 1200, "total_output_tokens": 300 },
            "cost": { "total_cost_usd": 0.37, "total_lines_added": 10, "total_lines_removed": 2 },
        }));
        assert!(state.has_status);
        assert_eq!(state.context_percent, Some(42.5));
        assert_eq!(state.total_tokens(), 1500);
        assert_eq!(state.cost_usd, Some(0.37));
        assert_eq!((state.lines_added, state.lines_removed), (10, 2));
        assert_eq!(state.model, "Opus");
        state.apply_status(&json!({ "context_window": { "used_percentage": 140.0, "current_usage": null } }));
        assert_eq!(state.context_percent, Some(100.0), "clamped");
    }

    #[test]
    fn tracks_quality_from_the_first_sample() {
        let mut state = GaugeState::default();
        assert_eq!(state.quality_delta(), None);
        state.apply_quality(QualitySample {
            errors: 2,
            warnings: 5,
            score: 85.0,
        });
        state.apply_quality(QualitySample {
            errors: 0,
            warnings: 7,
            score: 93.0,
        });
        assert_eq!(state.quality_delta(), Some((-2, 2)));
    }

    #[test]
    fn bands_percentages() {
        assert_eq!(band(10.0, 60, 85), Band::Good);
        assert_eq!(band(60.0, 60, 85), Band::Warn);
        assert_eq!(band(85.0, 60, 85), Band::Bad);
        assert_eq!(truncate("abcdef", 4), "abc…");
    }
}
