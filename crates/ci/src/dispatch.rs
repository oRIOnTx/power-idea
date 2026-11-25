//! Manual runs: which workflows have `on: workflow_dispatch`, their inputs (type, default,
//! required, choice options), validating the form, and which workflows count as deploys.

use serde_json::{Map, Value};

use crate::yaml::{self, Yaml};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputKind {
    String,
    Boolean,
    Number,
    Choice(Vec<String>),
    /// An environment name (free text; GitHub checks it).
    Environment,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchInput {
    pub name: String,
    pub description: String,
    pub required: bool,
    pub default: Option<String>,
    pub kind: InputKind,
}

/// `None`: the workflow cannot be started by hand. `Some(inputs)` (maybe empty): it can.
pub fn dispatch_inputs(workflow_yaml: &str) -> Option<Vec<DispatchInput>> {
    let doc = yaml::parse(workflow_yaml)?;
    // `on` is the trigger key; YAML 1.1 parsers read it as `true`, so some files quote it.
    let on = doc.get("on").or_else(|| doc.get("true"))?;
    let dispatch = match on {
        Yaml::Scalar(s) => return (s == "workflow_dispatch").then(Vec::new),
        Yaml::Seq(items) => {
            return items
                .iter()
                .any(|i| i.as_str() == Some("workflow_dispatch"))
                .then(Vec::new);
        }
        Yaml::Map(_) => on.get("workflow_dispatch")?,
        Yaml::Null => return None,
    };
    let Some(Yaml::Map(inputs)) = dispatch.get("inputs") else {
        return Some(Vec::new());
    };
    Some(inputs.iter().map(|(name, spec)| input(name, spec)).collect())
}

fn input(name: &str, spec: &Yaml) -> DispatchInput {
    let text = |key: &str| spec.get(key).and_then(Yaml::as_str).map(str::to_string);
    let kind = match text("type").as_deref() {
        Some("boolean") => InputKind::Boolean,
        Some("number") => InputKind::Number,
        Some("environment") => InputKind::Environment,
        Some("choice") => {
            let options = match spec.get("options") {
                Some(Yaml::Seq(items)) => items
                    .iter()
                    .filter_map(Yaml::as_str)
                    .map(str::to_string)
                    .collect(),
                _ => Vec::new(),
            };
            InputKind::Choice(options)
        }
        _ => InputKind::String,
    };
    DispatchInput {
        name: name.to_string(),
        description: text("description").unwrap_or_default(),
        required: spec.get("required").and_then(Yaml::as_bool).unwrap_or(false),
        default: text("default"),
        kind,
    }
}

/// The `inputs` object of a dispatch request; values as typed in the form (by input name).
/// Empty optional fields are left out so the workflow's defaults apply.
pub fn dispatch_body(
    inputs: &[DispatchInput],
    values: &[(String, String)],
) -> Result<Map<String, Value>, String> {
    let mut body = Map::new();
    for input in inputs {
        let value = values
            .iter()
            .find(|(name, _)| *name == input.name)
            .map_or_default(|(_, v)| v.trim());
        if value.is_empty() {
            if input.required && input.default.is_none() {
                return Err(format!("{} is required", input.name));
            }
            continue;
        }
        match &input.kind {
            InputKind::Number if value.parse::<f64>().is_err() => {
                return Err(format!("{} must be a number", input.name));
            }
            InputKind::Boolean if !matches!(value, "true" | "false") => {
                return Err(format!("{} must be true or false", input.name));
            }
            InputKind::Choice(options) if !options.is_empty() && !options.iter().any(|o| o == value) => {
                return Err(format!("{} must be one of: {}", input.name, options.join(", ")));
            }
            _ => {}
        }
        body.insert(input.name.clone(), Value::String(value.to_string()));
    }
    Ok(body)
}

/// Default name patterns of workflows that deploy (case-insensitive substrings).
pub const DEFAULT_DEPLOY_PATTERNS: &[&str] = &["deploy", "release", "publish", "prod"];

/// The workflow (by name or file path) deploys: starting or re-running it asks first.
pub fn is_deploy(name: &str, path: &str, patterns: &[String]) -> bool {
    let haystack = format!("{name} {}", path.rsplit('/').next().unwrap_or(path)).to_lowercase();
    patterns
        .iter()
        .map(|p| p.trim().to_lowercase())
        .any(|p| !p.is_empty() && haystack.contains(&p))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEPLOY: &str = r#"
name: Deploy
on:
  push:
    branches: [main]
  workflow_dispatch:
    inputs:
      environment:
        description: "Where to"
        type: choice
        required: true
        default: staging
        options: [staging, production]
      dry_run:
        type: boolean
        default: false
      replicas:
        type: number
      note:
        description: Free text
        required: true
jobs: {}
"#;

    #[test]
    fn reads_inputs_with_types_defaults_and_options() {
        let inputs = dispatch_inputs(DEPLOY).unwrap();
        let names: Vec<&str> = inputs.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["environment", "dry_run", "replicas", "note"]);
        assert_eq!(
            inputs[0].kind,
            InputKind::Choice(vec!["staging".into(), "production".into()])
        );
        assert!(inputs[0].required);
        assert_eq!(inputs[0].default.as_deref(), Some("staging"));
        assert_eq!(inputs[0].description, "Where to");
        assert_eq!(inputs[1].kind, InputKind::Boolean);
        assert_eq!(inputs[1].default.as_deref(), Some("false"));
        assert_eq!(inputs[2].kind, InputKind::Number);
        assert_eq!(inputs[3].kind, InputKind::String);
    }

    #[test]
    fn knows_which_workflows_can_be_started_by_hand() {
        assert_eq!(dispatch_inputs("on: workflow_dispatch\n"), Some(Vec::new()));
        assert_eq!(
            dispatch_inputs("on: [push, workflow_dispatch]\n"),
            Some(Vec::new())
        );
        assert_eq!(dispatch_inputs("on:\n  workflow_dispatch:\n"), Some(Vec::new()));
        assert_eq!(
            dispatch_inputs("\"on\":\n  workflow_dispatch: {}\n"),
            Some(Vec::new())
        );
        assert_eq!(dispatch_inputs("on: push\n"), None);
        assert_eq!(dispatch_inputs("on:\n  pull_request:\n"), None);
        assert_eq!(dispatch_inputs("name: x\n"), None);
    }

    #[test]
    fn validates_and_builds_the_body() {
        let inputs = dispatch_inputs(DEPLOY).unwrap();
        let values = |pairs: &[(&str, &str)]| -> Vec<(String, String)> {
            pairs.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect()
        };
        assert_eq!(
            dispatch_body(&inputs, &values(&[])),
            Err("note is required".into())
        );
        assert_eq!(
            dispatch_body(&inputs, &values(&[("note", "x"), ("environment", "moon")])),
            Err("environment must be one of: staging, production".into())
        );
        assert_eq!(
            dispatch_body(&inputs, &values(&[("note", "x"), ("replicas", "three")])),
            Err("replicas must be a number".into())
        );
        let body = dispatch_body(
            &inputs,
            &values(&[("note", " hi "), ("dry_run", "true"), ("replicas", "")]),
        )
        .unwrap();
        assert_eq!(
            Value::Object(body),
            serde_json::json!({"note": "hi", "dry_run": "true"})
        );
    }

    #[test]
    fn spots_deploy_workflows() {
        let patterns: Vec<String> = DEFAULT_DEPLOY_PATTERNS.iter().map(|s| (*s).to_string()).collect();
        assert!(is_deploy("Deploy to Fly", ".github/workflows/fly.yml", &patterns));
        assert!(is_deploy("CD", ".github/workflows/production.yml", &patterns));
        assert!(!is_deploy("CI", ".github/workflows/ci.yml", &patterns));
        assert!(is_deploy("Ship it", "x/ship.yml", &["SHIP".into()]));
        assert!(!is_deploy("Deploy", "d.yml", &[" ".into()]));
    }
}
