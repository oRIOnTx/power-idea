//! Results grid backed by gpui-component's virtualized table.

use std::collections::{BTreeMap, BTreeSet};

use db::edit::{CellEdit, TableRef, delete_statement, insert_statement, update_statement};
use db::{DbKind, ResultSet};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::table::{Column, TableDelegate, TableState};
use gpui_kit::*;

/// Shows one result set; NULLs are dimmed like in JetBrains. Pending changes
/// (edited cells, added and deleted rows) are kept here until submitted.
pub struct ResultsDelegate {
    pub set: ResultSet,
    /// Edited but not yet submitted cells: (row, column) → new value (`None` = NULL).
    pub edits: BTreeMap<(usize, usize), Option<String>>,
    /// Rows marked for deletion.
    pub deleted: BTreeSet<usize>,
    /// Rows from the database; rows after them were added here.
    original_rows: usize,
}

impl ResultsDelegate {
    pub fn new(set: ResultSet) -> Self {
        Self {
            original_rows: set.rows.len(),
            set,
            edits: BTreeMap::new(),
            deleted: BTreeSet::new(),
        }
    }

    pub fn is_new(&self, row: usize) -> bool {
        row >= self.original_rows
    }

    /// Appends an empty row; returns its index.
    pub fn add_row(&mut self) -> usize {
        self.set.rows.push(vec![None; self.set.columns.len()]);
        self.set.rows.len() - 1
    }

    pub fn toggle_delete(&mut self, row: usize) {
        if row < self.set.rows.len() && !self.deleted.remove(&row) {
            self.deleted.insert(row);
        }
    }

    pub fn revert(&mut self) {
        self.set.rows.truncate(self.original_rows);
        self.edits.clear();
        self.deleted.clear();
    }

    /// Statements that would be submitted, in order: deletes, updates, inserts.
    pub fn statements(&self, kind: DbKind, table: &TableRef, key: &[usize]) -> Vec<String> {
        // `None` when a key column or row is out of range: skipping the statement is safer
        // than a WHERE clause with fewer key columns than intended.
        let key_of = |row: usize| -> Option<Vec<(String, Option<String>)>> {
            let cells = self.set.rows.get(row)?;
            key.iter()
                .map(|&k| Some((self.set.columns.get(k)?.clone(), cells.get(k).cloned().flatten())))
                .collect()
        };
        let live = |row: usize| !self.deleted.contains(&row);
        let deletes = self
            .deleted
            .iter()
            .filter(|&&row| !self.is_new(row))
            .filter_map(|&row| Some(delete_statement(kind, table, &key_of(row)?)));
        let updates = self
            .edits
            .iter()
            .filter(|&(&(row, _), _)| !self.is_new(row) && live(row))
            .filter_map(|(&(row, col), value)| {
                let edit = CellEdit {
                    column: self.set.columns.get(col)?.clone(),
                    value: value.clone(),
                    key: key_of(row)?,
                };
                Some(update_statement(kind, table, &edit))
            });
        let inserts = (self.original_rows..self.set.rows.len())
            .filter(|&row| live(row))
            .map(|row| {
                let values: Vec<(String, Option<String>)> = self
                    .edits
                    .range((row, 0)..(row + 1, 0))
                    .filter_map(|(&(_, col), value)| {
                        Some((self.set.columns.get(col)?.clone(), value.clone()))
                    })
                    .collect();
                insert_statement(kind, table, &values)
            });
        deletes.chain(updates).chain(inserts).collect()
    }

    /// Number of changes waiting for Submit.
    pub fn pending(&self) -> usize {
        let live = |row: &usize| !self.deleted.contains(row);
        let edited = self
            .edits
            .keys()
            .filter(|(row, _)| !self.is_new(*row) && live(row))
            .count();
        let deleted = self.deleted.iter().filter(|&&row| !self.is_new(row)).count();
        let added = (self.original_rows..self.set.rows.len()).filter(live).count();
        edited + deleted + added
    }
}

impl TableDelegate for ResultsDelegate {
    fn columns_count(&self, _: &App) -> usize {
        self.set.columns.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.set.rows.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        let name = self.set.columns.get(col_ix).cloned().unwrap_or_default();
        Column::new(format!("c{col_ix}"), name)
            .width(px(160.))
            .min_width(px(60.))
            .resizable(true)
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let edited = self.edits.get(&(row_ix, col_ix));
        let cell = match edited {
            Some(value) => value.clone(),
            None => self
                .set
                .rows
                .get(row_ix)
                .and_then(|row| row.get(col_ix))
                .cloned()
                .flatten(),
        };
        let theme = cx.theme();
        let base = div()
            .w_full()
            .overflow_hidden()
            .text_ellipsis()
            .whitespace_nowrap();
        let base = if self.deleted.contains(&row_ix) {
            base.line_through().bg(theme.danger.opacity(0.14))
        } else if self.is_new(row_ix) {
            base.px_1().rounded_sm().bg(theme.green.opacity(0.16))
        } else if edited.is_some() {
            base.px_1().rounded_sm().bg(theme.warning.opacity(0.18))
        } else {
            base
        };
        match cell {
            Some(text) => base.child(text.replace('\n', "⏎")),
            None => base.italic().text_color(theme.muted_foreground).child("NULL"),
        }
    }

    fn cell_label(&self, row_ix: usize, col_ix: usize, _: &App) -> Option<SharedString> {
        self.set
            .rows
            .get(row_ix)?
            .get(col_ix)?
            .clone()
            .map(SharedString::from)
    }
}

#[cfg(test)]
mod tests {
    use db::edit::TableRef;
    use db::{DbKind, ResultSet};

    use super::ResultsDelegate;

    fn delegate() -> ResultsDelegate {
        ResultsDelegate::new(ResultSet {
            columns: vec!["id".into(), "name".into()],
            rows: vec![
                vec![Some("1".into()), Some("ann".into())],
                vec![Some("2".into()), Some("bob".into())],
            ],
            truncated: false,
        })
    }

    #[test]
    fn collects_pending_changes_as_statements() {
        let table = TableRef {
            schema: None,
            name: "users".into(),
        };
        let mut d = delegate();
        d.edits.insert((0, 1), Some("anna".into()));
        d.edits.insert((1, 1), Some("ignored".into()));
        d.toggle_delete(1);
        let new = d.add_row();
        d.edits.insert((new, 1), Some("cid".into()));
        let unused = d.add_row();
        d.toggle_delete(unused);
        assert_eq!(d.pending(), 3);
        assert_eq!(
            d.statements(DbKind::Sqlite, &table, &[0]),
            [
                "DELETE FROM \"users\" WHERE \"id\" = '2'",
                "UPDATE \"users\" SET \"name\" = 'anna' WHERE \"id\" = '1'",
                "INSERT INTO \"users\" (\"name\") VALUES ('cid')",
            ]
        );
        d.revert();
        assert_eq!((d.pending(), d.set.rows.len()), (0, 2));
    }
}
