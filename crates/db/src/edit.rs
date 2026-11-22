//! Editing result grids: finding the table a query reads, quoting for each
//! SQL dialect and building the UPDATE statements for edited cells.

use super::DbKind;

/// The single table behind a simple `SELECT … FROM table` query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableRef {
    pub schema: Option<String>,
    pub name: String,
}

/// One changed cell, identified by the primary-key values of its row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellEdit {
    pub column: String,
    /// `None` sets NULL.
    pub value: Option<String>,
    /// (key column, original value) of the row.
    pub key: Vec<(String, Option<String>)>,
}

/// Splits SQL into lowercase words and quoted identifiers, dropping comments
/// and string literals; enough to recognise simple single-table selects.
fn tokens(query: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = query.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '-' if chars.peek() == Some(&'-') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            '\'' => {
                for c in chars.by_ref() {
                    if c == '\'' {
                        break;
                    }
                }
                out.push("'".into());
            }
            '"' | '`' | '[' => {
                let close = if c == '[' { ']' } else { c };
                let ident: String = chars.by_ref().take_while(|&x| x != close).collect();
                out.push(format!("\u{1}{ident}"));
            }
            c if c.is_alphanumeric() || c == '_' || c == '$' => {
                let mut word = c.to_string();
                while let Some(&next) = chars.peek() {
                    if next.is_alphanumeric() || next == '_' || next == '$' {
                        word.push(next);
                        chars.next();
                    } else {
                        break;
                    }
                }
                out.push(word.to_lowercase());
            }
            c if c.is_whitespace() => {}
            c => out.push(c.to_string()),
        }
    }
    out
}

fn ident(token: &str) -> Option<String> {
    match token.strip_prefix('\u{1}') {
        Some(quoted) => Some(quoted.to_string()),
        None if token.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '$') => Some(token.into()),
        None => None,
    }
}

/// The table of `SELECT … FROM [schema.]table [WHERE …] [ORDER BY …] [LIMIT …]`;
/// `None` for joins, unions, grouping, functions, subqueries or several statements.
pub fn editable_table(query: &str) -> Option<TableRef> {
    let mut tokens = tokens(query);
    while tokens.last().is_some_and(|t| t == ";") {
        tokens.pop();
    }
    if tokens.first()? != "select" || tokens.iter().any(|t| t == ";") {
        return None;
    }
    const BLOCKERS: [&str; 9] = [
        "join",
        "union",
        "group",
        "having",
        "distinct",
        "intersect",
        "except",
        "with",
        "(",
    ];
    let from = tokens.iter().position(|t| t == "from")?;
    // Any parenthesis means functions or subqueries: the columns may not be plain table columns.
    if tokens.iter().any(|t| BLOCKERS.contains(&t.as_str())) {
        return None;
    }
    let first = ident(tokens.get(from + 1)?)?;
    let (schema, name, next) = if tokens.get(from + 2).map(String::as_str) == Some(".") {
        (Some(first), ident(tokens.get(from + 3)?)?, from + 4)
    } else {
        (None, first, from + 2)
    };
    // A comma after the table means several tables.
    if tokens.get(next).is_some_and(|t| t == ",") {
        return None;
    }
    Some(TableRef { schema, name })
}

/// Queries that only read, so they can be re-run for a full export.
pub fn is_read_only(query: &str) -> bool {
    let tokens = tokens(query);
    let statements = tokens.split(|t| t == ";").filter(|s| !s.is_empty()).count();
    statements == 1
        && matches!(
            tokens.first().map(String::as_str),
            Some("select" | "with" | "show" | "explain" | "values")
        )
        && !tokens.iter().any(|t| {
            matches!(
                t.as_str(),
                "insert" | "update" | "delete" | "into" | "drop" | "alter" | "create"
            )
        })
}

pub fn quote_ident(kind: DbKind, ident: &str) -> String {
    match kind {
        DbKind::MySql => format!("`{}`", ident.replace('`', "``")),
        _ => format!("\"{}\"", ident.replace('"', "\"\"")),
    }
}

/// A SQL literal for a text value (the database converts it to the column type).
pub fn quote_literal(kind: DbKind, value: Option<&str>) -> String {
    match value {
        None => "NULL".into(),
        // MySQL treats backslashes as escapes by default.
        Some(v) if kind == DbKind::MySql => format!("'{}'", v.replace('\\', "\\\\").replace('\'', "''")),
        Some(v) => format!("'{}'", v.replace('\'', "''")),
    }
}

pub fn qualified(kind: DbKind, table: &TableRef) -> String {
    match &table.schema {
        Some(schema) => format!("{}.{}", quote_ident(kind, schema), quote_ident(kind, &table.name)),
        None => quote_ident(kind, &table.name),
    }
}

/// `UPDATE table SET column = value WHERE key = original …` for one edited cell.
pub fn update_statement(kind: DbKind, table: &TableRef, edit: &CellEdit) -> String {
    format!(
        "UPDATE {} SET {} = {} WHERE {}",
        qualified(kind, table),
        quote_ident(kind, &edit.column),
        quote_literal(kind, edit.value.as_deref()),
        key_condition(kind, &edit.key)
    )
}

fn key_condition(kind: DbKind, key: &[(String, Option<String>)]) -> String {
    key.iter()
        .map(|(column, value)| match value {
            None => format!("{} IS NULL", quote_ident(kind, column)),
            Some(_) => format!(
                "{} = {}",
                quote_ident(kind, column),
                quote_literal(kind, value.as_deref())
            ),
        })
        .collect::<Vec<_>>()
        .join(" AND ")
}

/// `INSERT` of a new row; columns left out get their defaults.
pub fn insert_statement(kind: DbKind, table: &TableRef, values: &[(String, Option<String>)]) -> String {
    let table = qualified(kind, table);
    if values.is_empty() {
        return match kind {
            DbKind::MySql => format!("INSERT INTO {table} () VALUES ()"),
            _ => format!("INSERT INTO {table} DEFAULT VALUES"),
        };
    }
    let columns: Vec<String> = values.iter().map(|(c, _)| quote_ident(kind, c)).collect();
    let literals: Vec<String> = values
        .iter()
        .map(|(_, v)| quote_literal(kind, v.as_deref()))
        .collect();
    format!(
        "INSERT INTO {table} ({}) VALUES ({})",
        columns.join(", "),
        literals.join(", ")
    )
}

/// `DELETE` of the row with this primary key.
pub fn delete_statement(kind: DbKind, table: &TableRef, key: &[(String, Option<String>)]) -> String {
    format!(
        "DELETE FROM {} WHERE {}",
        qualified(kind, table),
        key_condition(kind, key)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(schema: Option<&str>, name: &str) -> Option<TableRef> {
        Some(TableRef {
            schema: schema.map(str::to_string),
            name: name.into(),
        })
    }

    #[test]
    fn finds_single_table_selects() {
        assert_eq!(
            editable_table("SELECT * FROM users LIMIT 100;"),
            table(None, "users")
        );
        assert_eq!(
            editable_table("select id, name from \"billing\".\"In voices\" where id > 3 order by id"),
            table(Some("billing"), "In voices")
        );
        assert_eq!(
            editable_table("SELECT * FROM `order` -- note\n"),
            table(None, "order")
        );
        assert_eq!(editable_table("SELECT * FROM a JOIN b ON a.id = b.a_id"), None);
        assert_eq!(editable_table("SELECT * FROM a, b"), None);
        assert_eq!(editable_table("SELECT count(*) FROM a"), None);
        assert_eq!(editable_table("SELECT * FROM a; DELETE FROM a"), None);
        assert_eq!(editable_table("UPDATE a SET x = 1"), None);
        assert_eq!(
            editable_table("SELECT * FROM a WHERE name = 'x join y'"),
            table(None, "a")
        );
    }

    #[test]
    fn detects_read_only_queries() {
        assert!(is_read_only("SELECT * FROM a"));
        assert!(is_read_only("with x as (select 1) select * from x;"));
        assert!(!is_read_only("SELECT 1; SELECT 2"));
        assert!(!is_read_only("DELETE FROM a"));
        assert!(!is_read_only("SELECT * INTO b FROM a"));
    }

    #[test]
    fn builds_updates_per_dialect() {
        let edit = CellEdit {
            column: "name".into(),
            value: Some("O'Brien \\ x".into()),
            key: vec![("id".into(), Some("7".into())), ("tenant".into(), None)],
        };
        assert_eq!(
            update_statement(
                DbKind::Postgres,
                &TableRef {
                    schema: Some("public".into()),
                    name: "users".into()
                },
                &edit
            ),
            "UPDATE \"public\".\"users\" SET \"name\" = 'O''Brien \\ x' WHERE \"id\" = '7' AND \"tenant\" IS NULL"
        );
        assert_eq!(
            update_statement(
                DbKind::MySql,
                &TableRef {
                    schema: None,
                    name: "users".into()
                },
                &edit
            ),
            "UPDATE `users` SET `name` = 'O''Brien \\\\ x' WHERE `id` = '7' AND `tenant` IS NULL"
        );
        let users = TableRef {
            schema: None,
            name: "users".into(),
        };
        assert_eq!(
            insert_statement(
                DbKind::Postgres,
                &users,
                &[("name".into(), Some("ann".into())), ("age".into(), None)]
            ),
            "INSERT INTO \"users\" (\"name\", \"age\") VALUES ('ann', NULL)"
        );
        assert_eq!(
            insert_statement(DbKind::MySql, &users, &[]),
            "INSERT INTO `users` () VALUES ()"
        );
        assert_eq!(
            insert_statement(DbKind::Sqlite, &users, &[]),
            "INSERT INTO \"users\" DEFAULT VALUES"
        );
        assert_eq!(
            delete_statement(DbKind::Sqlite, &users, &edit.key),
            "DELETE FROM \"users\" WHERE \"id\" = '7' AND \"tenant\" IS NULL"
        );
        let null = CellEdit { value: None, ..edit };
        assert!(
            update_statement(
                DbKind::Sqlite,
                &TableRef {
                    schema: None,
                    name: "t".into()
                },
                &null
            )
            .contains("SET \"name\" = NULL")
        );
    }
}
