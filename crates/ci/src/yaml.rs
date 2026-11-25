//! Just enough YAML for workflow files: tree-sitter's syntax tree folded into plain values
//! (mappings, sequences, scalars). Anchors, aliases and tags are not resolved.

use tree_sitter::{Node, Parser};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Yaml {
    Map(Vec<(String, Yaml)>),
    Seq(Vec<Yaml>),
    Scalar(String),
    Null,
}

impl Yaml {
    pub fn get(&self, key: &str) -> Option<&Yaml> {
        match self {
            Self::Map(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Scalar(s) => Some(s),
            _ => None,
        }
    }

    /// `true` / `false` in any of YAML 1.1's spellings GitHub accepts.
    pub fn as_bool(&self) -> Option<bool> {
        match self.as_str()?.to_ascii_lowercase().as_str() {
            "true" | "yes" | "on" => Some(true),
            "false" | "no" | "off" => Some(false),
            _ => None,
        }
    }
}

/// The first document of `source`, or `None` when it does not parse.
pub fn parse(source: &str) -> Option<Yaml> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_yaml::LANGUAGE.into()).ok()?;
    let tree = parser.parse(source, None)?;
    let root = tree.root_node();
    if root.has_error() {
        return None;
    }
    let mut cursor = root.walk();
    let document = root
        .named_children(&mut cursor)
        .find(|n| n.kind() == "document")?;
    let mut cursor = document.walk();
    let body = document.named_children(&mut cursor).find(|n| is_node(n.kind()))?;
    Some(value(body, source.as_bytes()))
}

fn is_node(kind: &str) -> bool {
    matches!(kind, "block_node" | "flow_node")
}

fn value(node: Node<'_>, src: &[u8]) -> Yaml {
    match node.kind() {
        "block_node" | "flow_node" => {
            let mut cursor = node.walk();
            // Skip anchors and tags; the content is the last named child.
            let content = node
                .named_children(&mut cursor)
                .filter(|c| !matches!(c.kind(), "anchor" | "tag" | "comment"))
                .last();
            content.map_or(Yaml::Null, |c| value(c, src))
        }
        "block_mapping" | "flow_mapping" => {
            let mut cursor = node.walk();
            let pairs = node
                .named_children(&mut cursor)
                .filter(|c| matches!(c.kind(), "block_mapping_pair" | "flow_pair"))
                .filter_map(|pair| {
                    let key = pair.child_by_field_name("key").map(|k| value(k, src))?;
                    let key = key.as_str()?.to_string();
                    let value = pair
                        .child_by_field_name("value")
                        .map_or(Yaml::Null, |v| value(v, src));
                    Some((key, value))
                })
                .collect();
            Yaml::Map(pairs)
        }
        "block_sequence" | "flow_sequence" => {
            let mut cursor = node.walk();
            let items = node
                .named_children(&mut cursor)
                .filter(|c| c.kind() != "comment")
                .map(|item| {
                    if item.kind() == "block_sequence_item" {
                        let mut cursor = item.walk();
                        let inner = item.named_children(&mut cursor).find(|n| is_node(n.kind()));
                        inner.map_or(Yaml::Null, |n| value(n, src))
                    } else {
                        value(item, src)
                    }
                })
                .collect();
            Yaml::Seq(items)
        }
        "plain_scalar" => {
            let text = text(node, src);
            if matches!(text.as_str(), "~" | "null" | "Null" | "NULL") {
                Yaml::Null
            } else {
                Yaml::Scalar(text)
            }
        }
        "double_quote_scalar" => Yaml::Scalar(unquote_double(&text(node, src))),
        "single_quote_scalar" => {
            let raw = text(node, src);
            let inner = raw.get(1..raw.len().saturating_sub(1)).unwrap_or_default();
            Yaml::Scalar(inner.replace("''", "'"))
        }
        "block_scalar" => Yaml::Scalar(block_scalar(&text(node, src))),
        _ => Yaml::Null,
    }
}

fn text(node: Node<'_>, src: &[u8]) -> String {
    node.utf8_text(src).unwrap_or_default().trim().to_string()
}

fn unquote_double(raw: &str) -> String {
    let inner = raw.get(1..raw.len().saturating_sub(1)).unwrap_or_default();
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

/// `|` / `>` scalars: the indented lines without their common indentation (folding ignored).
fn block_scalar(raw: &str) -> String {
    let mut lines = raw.lines();
    lines.next(); // the `|` / `>` header
    let body: Vec<&str> = lines.collect();
    let indent = body
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    body.iter()
        .map(|l| l.get(indent..).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_mappings_sequences_and_scalars() {
        let yaml = parse(
            "name: CI # comment\non:\n  push:\n    branches: [main, 'release/*']\n  workflow_dispatch:\n\
             env:\n  A: \"x\\ny\"\n  B: ~\nsteps:\n  - run: |\n      echo 1\n      echo 2\n  - uses: x\n",
        )
        .unwrap();
        assert_eq!(yaml.get("name").and_then(Yaml::as_str), Some("CI"));
        let on = yaml.get("on").unwrap();
        assert_eq!(
            on.get("push").and_then(|p| p.get("branches")),
            Some(&Yaml::Seq(vec![
                Yaml::Scalar("main".into()),
                Yaml::Scalar("release/*".into())
            ]))
        );
        assert_eq!(on.get("workflow_dispatch"), Some(&Yaml::Null));
        assert_eq!(
            yaml.get("env").and_then(|e| e.get("A")).and_then(Yaml::as_str),
            Some("x\ny")
        );
        let Some(Yaml::Seq(steps)) = yaml.get("steps") else {
            panic!()
        };
        assert_eq!(steps[0].get("run").and_then(Yaml::as_str), Some("echo 1\necho 2"));
    }

    #[test]
    fn rejects_broken_yaml() {
        assert_eq!(parse("on: [push\nname: x: y: : ["), None);
    }
}
