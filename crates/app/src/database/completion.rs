//! Completion in SQL consoles: keywords, tables and columns from the
//! connection's schema; `table.` lists that table's columns.

use std::collections::BTreeSet;

use anyhow::Result;
use db::{NodeKind, SchemaNode};
use gpui_kit::base::input::Rope;
use gpui_kit::component::input::{CompletionProvider, RopeExt as _};
use gpui_kit::*;
use lsp::lsp_types::{self, CompletionItemKind};

const MAX_ITEMS: usize = 100;

const KEYWORDS: &[&str] = &[
    "SELECT",
    "FROM",
    "WHERE",
    "AND",
    "OR",
    "NOT",
    "NULL",
    "IS",
    "IN",
    "LIKE",
    "ILIKE",
    "BETWEEN",
    "ORDER BY",
    "GROUP BY",
    "HAVING",
    "LIMIT",
    "OFFSET",
    "JOIN",
    "LEFT JOIN",
    "INNER JOIN",
    "ON",
    "AS",
    "DISTINCT",
    "COUNT",
    "INSERT INTO",
    "VALUES",
    "UPDATE",
    "SET",
    "DELETE FROM",
    "CREATE TABLE",
    "ALTER TABLE",
    "DROP TABLE",
    "UNION",
    "CASE",
    "WHEN",
    "THEN",
    "ELSE",
    "END",
    "EXISTS",
    "RETURNING",
    "WITH",
    "EXPLAIN",
    "DESC",
    "ASC",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CandidateKind {
    Column,
    Table,
    Keyword,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Candidate {
    pub kind: CandidateKind,
    pub label: String,
    /// Column type or the table a column belongs to.
    pub detail: Option<String>,
}

fn tables(schema: &[SchemaNode]) -> impl Iterator<Item = &SchemaNode> {
    schema
        .iter()
        .flat_map(|s| s.children.iter())
        .filter(|t| matches!(t.kind, NodeKind::Table | NodeKind::View))
}

fn columns(table: &SchemaNode) -> impl Iterator<Item = Candidate> + '_ {
    table.children.iter().filter_map(|c| match &c.kind {
        NodeKind::Column { data_type } => Some(Candidate {
            kind: CandidateKind::Column,
            label: c.name.clone(),
            detail: Some(data_type.clone()),
        }),
        _ => None,
    })
}

/// The identifier being typed at the end of `before` and, after `x.`, the qualifier `x`.
pub fn word_before(before: &str) -> (&str, Option<&str>) {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let start = before.rfind(|c: char| !is_word(c)).map_or(0, |ix| ix + 1);
    let word = &before[start..];
    let qualifier = before[..start].strip_suffix('.').map(|head| {
        let q_start = head.rfind(|c: char| !is_word(c)).map_or(0, |ix| ix + 1);
        &head[q_start..]
    });
    (word, qualifier.filter(|q| !q.is_empty()))
}

/// Candidates for the text before the cursor, best matches first.
pub fn sql_completions(schema: &[SchemaNode], before: &str) -> Vec<Candidate> {
    let (word, qualifier) = word_before(before);
    let prefix = word.to_lowercase();
    let matches = |label: &str| label.to_lowercase().starts_with(&prefix);

    if let Some(qualifier) = qualifier {
        let qualifier = qualifier.to_lowercase();
        return tables(schema)
            .filter(|t| t.name.to_lowercase() == qualifier)
            .flat_map(columns)
            .filter(|c| matches(&c.label))
            .take(MAX_ITEMS)
            .collect();
    }
    if prefix.is_empty() {
        return Vec::new();
    }
    let mut found = BTreeSet::new();
    for table in tables(schema) {
        if matches(&table.name) {
            found.insert(Candidate {
                kind: CandidateKind::Table,
                label: table.name.clone(),
                detail: None,
            });
        }
        for mut column in columns(table).filter(|c| matches(&c.label)) {
            column.detail = Some(table.name.clone());
            found.insert(column);
        }
    }
    found.extend(KEYWORDS.iter().filter(|k| matches(k)).map(|k| Candidate {
        kind: CandidateKind::Keyword,
        label: (*k).to_string(),
        detail: None,
    }));
    found.into_iter().take(MAX_ITEMS).collect()
}

/// Completion provider installed on a console editor.
pub struct SqlCompletion {
    pub schema: Vec<SchemaNode>,
}

impl CompletionProvider for SqlCompletion {
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        _: lsp_types::CompletionContext,
        _: &mut Window,
        _: &mut App,
    ) -> Task<Result<lsp_types::CompletionResponse>> {
        let all = text.to_string();
        let before = all.get(..offset).unwrap_or(&all);
        let line = before.rsplit('\n').next().unwrap_or(before);
        let (word, _) = word_before(line);
        // Replace the typed prefix with the chosen item.
        let range = lsp_types::Range::new(
            text.offset_to_position(offset - word.len()),
            text.offset_to_position(offset),
        );
        let items = sql_completions(&self.schema, line)
            .into_iter()
            .map(|c| lsp_types::CompletionItem {
                kind: Some(match c.kind {
                    CandidateKind::Column => CompletionItemKind::FIELD,
                    CandidateKind::Table => CompletionItemKind::CLASS,
                    CandidateKind::Keyword => CompletionItemKind::KEYWORD,
                }),
                detail: c.detail,
                text_edit: Some(lsp_types::CompletionTextEdit::Edit(lsp_types::TextEdit {
                    range,
                    new_text: c.label.clone(),
                })),
                label: c.label,
                ..Default::default()
            })
            .collect();
        Task::ready(Ok(lsp_types::CompletionResponse::Array(items)))
    }

    fn is_completion_trigger(&self, _: usize, new_text: &str, _: &mut App) -> bool {
        new_text
            .chars()
            .any(|c| c.is_alphanumeric() || c == '_' || c == '.')
    }
}

#[cfg(test)]
mod tests {
    use db::{NodeKind, SchemaNode};

    use super::{CandidateKind, sql_completions, word_before};

    fn schema() -> Vec<SchemaNode> {
        let column = |name: &str, ty: &str| SchemaNode {
            name: name.into(),
            kind: NodeKind::Column { data_type: ty.into() },
            children: Vec::new(),
        };
        let table = |name: &str, columns| SchemaNode {
            name: name.into(),
            kind: NodeKind::Table,
            children: columns,
        };
        vec![SchemaNode {
            name: "public".into(),
            kind: NodeKind::Schema,
            children: vec![
                table("users", vec![column("id", "int"), column("email", "text")]),
                table("user_roles", vec![column("user_id", "int")]),
            ],
        }]
    }

    #[test]
    fn splits_word_and_qualifier() {
        assert_eq!(word_before("select u"), ("u", None));
        assert_eq!(word_before("select users.em"), ("em", Some("users")));
        assert_eq!(word_before("select users."), ("", Some("users")));
        assert_eq!(word_before("select "), ("", None));
    }

    #[test]
    fn completes_tables_columns_and_keywords() {
        let labels = |before| -> Vec<(CandidateKind, String)> {
            sql_completions(&schema(), before)
                .into_iter()
                .map(|c| (c.kind, c.label))
                .collect()
        };
        assert_eq!(
            labels("SELECT * FROM us"),
            [
                (CandidateKind::Column, "user_id".into()),
                (CandidateKind::Table, "user_roles".into()),
                (CandidateKind::Table, "users".into()),
            ]
        );
        assert_eq!(
            labels("select USERS."),
            [
                (CandidateKind::Column, "id".into()),
                (CandidateKind::Column, "email".into())
            ]
        );
        assert!(labels("sel").contains(&(CandidateKind::Keyword, "SELECT".into())));
        assert!(labels("select ").is_empty());
    }
}
