//! Messages of the extension protocol (see `docs/extensions.md`) and the position
//! arithmetic shared by the IDE side. Positions are 1-based; columns count characters.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Bumped on incompatible protocol changes; sent in `initialize`.
pub const API_VERSION: u32 = 1;

/// IDE → extension: the handshake request.
pub const INITIALIZE: &str = "initialize";
/// IDE → extension: run a command from the manifest (request).
pub const EXECUTE_COMMAND: &str = "command/execute";
/// IDE → extension: something happened in the project (notification).
pub const EVENT: &str = "event";
/// IDE → extension: the project window is closing; exit now (notification).
pub const SHUTDOWN: &str = "shutdown";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    pub line: u32,
    pub column: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum PickItem {
    Label(String),
    Detailed {
        label: String,
        #[serde(default)]
        detail: Option<String>,
    },
}

impl PickItem {
    pub fn label(&self) -> &str {
        match self {
            Self::Label(label) | Self::Detailed { label, .. } => label,
        }
    }

    pub fn detail(&self) -> Option<&str> {
        match self {
            Self::Label(_) => None,
            Self::Detailed { detail, .. } => detail.as_deref(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    #[default]
    Info,
    Success,
    Warning,
    Error,
}

/// Extension → IDE requests.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "method", content = "params", deny_unknown_fields)]
pub enum HostRequest {
    /// Project root, name, git branch, open files and the active one.
    #[serde(rename = "workspace/info")]
    WorkspaceInfo {},
    /// Opens a project file, optionally at a position.
    #[serde(rename = "workspace/openFile")]
    OpenFile {
        path: String,
        #[serde(default)]
        line: Option<u32>,
        #[serde(default)]
        column: Option<u32>,
    },
    /// The active editor: path, language, cursor, selection (or null).
    #[serde(rename = "editor/active")]
    ActiveEditor {},
    /// Text of the active editor, or of a project file (open buffer first).
    #[serde(rename = "editor/text")]
    Text {
        #[serde(default)]
        path: Option<String>,
    },
    /// Replaces `range` (default: the selection) with `text` as one undoable edit.
    #[serde(rename = "editor/edit")]
    Edit {
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        range: Option<Range>,
        text: String,
    },
    /// Moves the cursor to `start`, selecting up to `end` when given.
    #[serde(rename = "editor/select")]
    Select {
        #[serde(default)]
        path: Option<String>,
        start: Position,
        #[serde(default)]
        end: Option<Position>,
    },
    #[serde(rename = "ui/notify")]
    Notify {
        message: String,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        level: Level,
    },
    /// A picker like Find Action; answers the chosen index or null.
    #[serde(rename = "ui/pick")]
    Pick {
        items: Vec<PickItem>,
        #[serde(default)]
        placeholder: Option<String>,
    },
    /// A one-line input dialog; answers the text or null.
    #[serde(rename = "ui/input")]
    Input {
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        value: Option<String>,
    },
    /// Runs a shell command in its own terminal tab.
    #[serde(rename = "terminal/run")]
    RunInTerminal {
        command: String,
        #[serde(default)]
        title: Option<String>,
        /// Relative to the project root.
        #[serde(default)]
        cwd: Option<String>,
    },
}

impl HostRequest {
    pub fn parse(method: &str, params: Value) -> Result<Self, String> {
        let params = if params.is_null() { json!({}) } else { params };
        serde_json::from_value(json!({ "method": method, "params": params })).map_err(|err| {
            let text = err.to_string();
            if text.starts_with("unknown variant") {
                format!("unknown method {method}")
            } else {
                format!("{method}: {text}")
            }
        })
    }
}

/// `initialize` params.
pub fn initialize_params(project_root: &str, project_name: &str, extension_id: &str, dir: &str) -> Value {
    json!({
        "apiVersion": API_VERSION,
        "ide": { "name": "Power IDEA", "version": env!("CARGO_PKG_VERSION") },
        "project": { "root": project_root, "name": project_name },
        "extension": { "id": extension_id, "dir": dir },
    })
}

/// `event` params, e.g. `{"type": "file_saved", "path": "src/main.rs"}`.
pub fn event_params(kind: &str, path: &str) -> Value {
    json!({ "type": kind, "path": path })
}

/// Byte offset of a 1-based position; columns past the line end clamp to it.
/// `None` when the line does not exist.
pub fn offset_of(text: &str, position: Position) -> Option<usize> {
    let line_ix = position.line.checked_sub(1)? as usize;
    let mut start = 0;
    for _ in 0..line_ix {
        start += text[start..].find('\n')? + 1;
    }
    let line = &text[start..];
    let line = &line[..line.find('\n').unwrap_or(line.len())];
    let column = position.column.saturating_sub(1) as usize;
    let within = line.char_indices().nth(column).map_or(line.len(), |(ix, _)| ix);
    Some(start + within)
}

/// 1-based position of a byte offset (clamped to the text, snapped to a char boundary).
pub fn position_of(text: &str, offset: usize) -> Position {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    let before = &text[..offset];
    let line_start = before.rfind('\n').map_or(0, |ix| ix + 1);
    Position {
        line: before.matches('\n').count() as u32 + 1,
        column: before[line_start..].chars().count() as u32 + 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(line: u32, column: u32) -> Position {
        Position { line, column }
    }

    #[test]
    fn converts_positions_and_offsets() {
        let text = "fn main() {\n    ünï\n}";
        assert_eq!(offset_of(text, pos(1, 1)), Some(0));
        assert_eq!(offset_of(text, pos(2, 5)), Some(16));
        assert_eq!(offset_of(text, pos(2, 6)), Some(18), "ü is two bytes");
        assert_eq!(offset_of(text, pos(2, 99)), Some(21), "clamps to the line end");
        assert_eq!(offset_of(text, pos(3, 2)), Some(23));
        assert_eq!(offset_of(text, pos(4, 1)), None);
        assert_eq!(offset_of(text, pos(0, 1)), None);
        for offset in [0, 12, 16, 18, 21, 22, 23] {
            assert_eq!(offset_of(text, position_of(text, offset)), Some(offset));
        }
        assert_eq!(position_of(text, 17), pos(2, 5), "snaps inside ü");
        assert_eq!(position_of(text, 999), pos(3, 2));
    }

    #[test]
    fn parses_requests() {
        assert_eq!(
            HostRequest::parse("workspace/info", Value::Null),
            Ok(HostRequest::WorkspaceInfo {})
        );
        let edit = HostRequest::parse(
            "editor/edit",
            json!({ "text": "x", "range": { "start": { "line": 1, "column": 1 }, "end": { "line": 1, "column": 3 } } }),
        );
        assert_eq!(
            edit,
            Ok(HostRequest::Edit {
                path: None,
                range: Some(Range {
                    start: pos(1, 1),
                    end: pos(1, 3)
                }),
                text: "x".into(),
            })
        );
        let pick = HostRequest::parse(
            "ui/pick",
            json!({ "items": ["a", { "label": "b", "detail": "B" }] }),
        );
        let Ok(HostRequest::Pick { items, .. }) = pick else {
            panic!("{pick:?}")
        };
        assert_eq!((items[1].label(), items[1].detail()), ("b", Some("B")));
        assert_eq!(
            HostRequest::parse("ui/notify", json!({ "message": "hi", "level": "warning" })),
            Ok(HostRequest::Notify {
                message: "hi".into(),
                title: None,
                level: Level::Warning
            })
        );
    }

    #[test]
    fn reports_bad_requests() {
        assert_eq!(
            HostRequest::parse("fs/delete", json!({})),
            Err("unknown method fs/delete".into())
        );
        let missing = HostRequest::parse("editor/edit", json!({})).unwrap_err();
        assert!(missing.contains("missing field `text`"), "{missing}");
    }
}
