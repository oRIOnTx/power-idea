//! Typed views of the DAP responses the UI shows: stack frames, scopes,
//! variables and program output.

use std::path::PathBuf;

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackFrame {
    pub id: i64,
    pub name: String,
    /// Local source file; `None` for frames without source (native code).
    pub path: Option<PathBuf>,
    /// 1-based.
    pub line: u32,
    pub column: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub name: String,
    pub reference: i64,
    /// The adapter warns that loading it is slow (e.g. globals).
    pub expensive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variable {
    pub name: String,
    pub value: String,
    pub type_name: Option<String>,
    /// Non-zero when the variable has children to expand.
    pub reference: i64,
}

fn str_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn int_field(value: &Value, key: &str) -> i64 {
    value.get(key).and_then(Value::as_i64).unwrap_or(0)
}

/// `stackTrace` response body.
pub fn stack_frames(body: &Value) -> Vec<StackFrame> {
    body.get("stackFrames")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|frame| StackFrame {
            id: int_field(frame, "id"),
            name: str_field(frame, "name"),
            path: frame
                .pointer("/source/path")
                .and_then(Value::as_str)
                .filter(|p| !p.is_empty())
                .map(PathBuf::from),
            line: int_field(frame, "line").max(0) as u32,
            column: int_field(frame, "column").max(0) as u32,
        })
        .collect()
}

/// `scopes` response body.
pub fn scopes(body: &Value) -> Vec<Scope> {
    body.get("scopes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|scope| Scope {
            name: str_field(scope, "name"),
            reference: int_field(scope, "variablesReference"),
            expensive: scope.get("expensive").and_then(Value::as_bool).unwrap_or(false),
        })
        .collect()
}

/// `variables` response body.
pub fn variables(body: &Value) -> Vec<Variable> {
    body.get("variables")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|variable| Variable {
            name: str_field(variable, "name"),
            value: str_field(variable, "value"),
            type_name: variable
                .get("type")
                .and_then(Value::as_str)
                .filter(|t| !t.is_empty())
                .map(str::to_string),
            reference: int_field(variable, "variablesReference"),
        })
        .collect()
}

/// Text of an `output` event worth showing (telemetry is dropped).
pub fn output_text(body: &Value) -> Option<String> {
    let category = body.get("category").and_then(Value::as_str).unwrap_or("console");
    if category == "telemetry" {
        return None;
    }
    body.get("output").and_then(Value::as_str).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use super::{output_text, scopes, stack_frames, variables};

    #[test]
    fn reads_responses() {
        let frames = stack_frames(&json!({ "stackFrames": [
            { "id": 7, "name": "User#save", "line": 12, "column": 3, "source": { "path": "/p/app/models/user.rb" } },
            { "id": 8, "name": "<native>", "line": 0, "column": 0 }
        ]}));
        assert_eq!(frames[0].path, Some(PathBuf::from("/p/app/models/user.rb")));
        assert_eq!((frames[0].id, frames[0].line), (7, 12));
        assert_eq!(frames[1].path, None);

        let scopes = scopes(&json!({ "scopes": [
            { "name": "Locals", "variablesReference": 3 },
            { "name": "Globals", "variablesReference": 4, "expensive": true }
        ]}));
        assert_eq!((scopes[0].reference, scopes[1].expensive), (3, true));

        let vars = variables(&json!({ "variables": [
            { "name": "user", "value": "#<User id: 1>", "type": "User", "variablesReference": 9 },
            { "name": "x", "value": "41", "variablesReference": 0 }
        ]}));
        assert_eq!(vars[0].type_name.as_deref(), Some("User"));
        assert_eq!((vars[1].name.as_str(), vars[1].reference), ("x", 0));

        assert_eq!(
            output_text(&json!({ "category": "stdout", "output": "hi\n" })).as_deref(),
            Some("hi\n")
        );
        assert_eq!(
            output_text(&json!({ "category": "telemetry", "output": "x" })),
            None
        );
    }
}
