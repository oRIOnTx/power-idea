//! YAML front-matter of Markdown files (rules, SKILL.md): split it off the body and turn it
//! into JSON, so serde can validate it with field-level errors.

use std::fmt::Write as _;

use anyhow::{Context as _, Result, bail};
use serde_json::{Map, Number, Value};
use yaml_rust2::{Yaml, YamlLoader};

/// A Markdown file split into its front-matter (`---` … `---`) and body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document<'a> {
    /// The YAML between the fences; empty when the file has none.
    pub front_matter: &'a str,
    pub body: &'a str,
}

/// Splits `text` at its leading `---` fences; text without them is all body.
pub fn split(text: &str) -> Document<'_> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let Some(rest) = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
    else {
        return Document {
            front_matter: "",
            body: text,
        };
    };
    // The closing fence is a line that is exactly `---`.
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end_matches(['\r', '\n']) == "---" {
            let front_matter = rest.get(..offset).unwrap_or_default();
            let body = rest.get(offset + line.len()..).unwrap_or_default();
            return Document {
                front_matter,
                body: body.strip_prefix('\n').unwrap_or(body),
            };
        }
        offset += line.len();
    }
    // No closing fence: treat everything as body rather than guessing.
    Document {
        front_matter: "",
        body: text,
    }
}

/// Parses front-matter YAML into a JSON object (`{}` when empty).
pub fn to_json(yaml: &str) -> Result<Value> {
    if yaml.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    let docs = YamlLoader::load_from_str(yaml).context("invalid YAML")?;
    let Some(doc) = docs.into_iter().next() else {
        return Ok(Value::Object(Map::new()));
    };
    let value = yaml_to_json(doc)?;
    if !value.is_object() {
        bail!("front-matter must be a mapping of `key: value` lines");
    }
    Ok(value)
}

fn yaml_to_json(yaml: Yaml) -> Result<Value> {
    Ok(match yaml {
        Yaml::Null | Yaml::BadValue => Value::Null,
        Yaml::Boolean(b) => Value::Bool(b),
        Yaml::Integer(i) => Value::Number(i.into()),
        Yaml::Real(text) => {
            let number = text.parse::<f64>().ok().and_then(Number::from_f64);
            number.map_or(Value::String(text), Value::Number)
        }
        Yaml::String(s) => Value::String(s),
        Yaml::Array(items) => Value::Array(items.into_iter().map(yaml_to_json).collect::<Result<_>>()?),
        Yaml::Hash(hash) => {
            let mut map = Map::new();
            for (key, value) in hash {
                let key = match key {
                    Yaml::String(s) => s,
                    Yaml::Integer(i) => i.to_string(),
                    Yaml::Boolean(b) => b.to_string(),
                    other => bail!("unsupported key {other:?}"),
                };
                map.insert(key, yaml_to_json(value)?);
            }
            Value::Object(map)
        }
        Yaml::Alias(_) => bail!("YAML aliases are not supported"),
    })
}

/// Renders `meta` (a JSON object) as YAML front-matter lines, keys in the given order.
/// Strings are always quoted, so values like `yes` or `1.0` round-trip as strings.
pub fn render(meta: &[(&str, Value)]) -> String {
    let mut out = String::new();
    for (key, value) in meta {
        render_entry(&mut out, key, value, 0);
    }
    out
}

fn render_entry(out: &mut String, key: &str, value: &Value, indent: usize) {
    let pad = "  ".repeat(indent);
    match value {
        Value::Object(map) => {
            let _ = writeln!(out, "{pad}{key}:");
            for (k, v) in map {
                render_entry(out, k, v, indent + 1);
            }
        }
        other => {
            let _ = writeln!(out, "{pad}{key}: {}", scalar(other));
        }
    }
}

fn scalar(value: &Value) -> String {
    match value {
        Value::Array(items) => format!("[{}]", items.iter().map(scalar).collect::<Vec<_>>().join(", ")),
        // JSON string escapes are valid in YAML double-quoted scalars.
        Value::String(s) => Value::String(s.clone()).to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn splits_front_matter() {
        let doc = split("---\nname: A\n---\n\n# Body\n---\nnot a fence");
        assert_eq!(doc.front_matter, "name: A\n");
        assert_eq!(doc.body, "# Body\n---\nnot a fence");
        assert_eq!(split("# No front-matter").body, "# No front-matter");
        assert_eq!(split("---\nname: unclosed").front_matter, "");
        assert_eq!(
            split("\u{feff}---\r\nname: A\r\n---\r\nB").front_matter,
            "name: A\r\n"
        );
    }

    #[test]
    fn converts_yaml_to_json() {
        let yaml = "name: Rust style\nenabled: true\npriority: 10\nratio: 0.5\ntags: [style, rust]\n\
                    applies_to:\n  languages:\n    - rust\n  paths: [\"src/**\"]\nnothing:\n";
        assert_eq!(
            to_json(yaml).unwrap(),
            json!({
                "name": "Rust style", "enabled": true, "priority": 10, "ratio": 0.5,
                "tags": ["style", "rust"],
                "applies_to": { "languages": ["rust"], "paths": ["src/**"] },
                "nothing": null,
            })
        );
        assert_eq!(to_json("").unwrap(), json!({}));
        assert!(to_json("- a\n- b").is_err(), "a list is not a mapping");
        assert!(to_json("name: [unclosed").is_err());
    }

    #[test]
    fn renders_and_round_trips() {
        let meta = [
            ("name", json!("Yes: really")),
            ("enabled", json!(true)),
            ("priority", json!(5)),
            ("tags", json!(["a", "b c"])),
            ("applies_to", json!({ "languages": ["rust"] })),
        ];
        let yaml = render(&meta);
        assert_eq!(
            yaml,
            "name: \"Yes: really\"\nenabled: true\npriority: 5\ntags: [\"a\", \"b c\"]\napplies_to:\n  languages: [\"rust\"]\n"
        );
        let back = to_json(&yaml).unwrap();
        assert_eq!(back["name"], "Yes: really");
        assert_eq!(back["applies_to"]["languages"], json!(["rust"]));
    }
}
