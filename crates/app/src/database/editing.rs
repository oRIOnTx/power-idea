//! Editing and exporting console results. Results of a single-table SELECT
//! that includes the primary key can be edited (double-click a cell), and rows
//! added or deleted; changes stay pending, highlighted, until Submit.

use std::path::PathBuf;

use db::edit::{TableRef, editable_table, is_read_only};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Textarea, TextareaState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::*;

use super::console::QueryConsole;

/// Export re-runs read-only queries without the grid's row limit, up to this many rows.
const EXPORT_ROW_LIMIT: usize = 1_000_000;

/// Whether the current results can be edited, and how rows are identified.
pub enum Editability {
    Unknown,
    ReadOnly(String),
    Editable {
        table: TableRef,
        /// Indices of the primary-key columns in the result set.
        key: Vec<usize>,
    },
}

impl QueryConsole {
    /// Looks up the primary key of the queried table to decide if results are editable.
    pub(super) fn detect_editability(&mut self, cx: &mut Context<Self>) {
        let Some(columns) = self
            .table
            .as_ref()
            .map(|t| t.read(cx).delegate().set.columns.clone())
        else {
            self.editability = Editability::Unknown;
            return;
        };
        let Some(table) = editable_table(&self.last_query).filter(|_| self.kind.is_sql()) else {
            self.editability = Editability::ReadOnly("Read-only: results are not from a single table".into());
            return;
        };
        self.editability = Editability::Unknown;
        let db = self.db.clone();
        cx.spawn(async move |this, cx| {
            let lookup = table.clone();
            let key = cx.background_spawn(async move { db.primary_key(&lookup) }).await;
            this.update(cx, |this, cx| {
                this.editability = match key {
                    Ok(key) if key.is_empty() => {
                        Editability::ReadOnly(format!("Read-only: {} has no primary key", table.name))
                    }
                    Ok(key) => match key
                        .iter()
                        .map(|k| columns.iter().position(|c| c == k))
                        .collect::<Option<Vec<_>>>()
                    {
                        Some(key) => Editability::Editable { table, key },
                        None => {
                            Editability::ReadOnly(format!("Read-only: select {} to edit", key.join(", ")))
                        }
                    },
                    Err(err) => Editability::ReadOnly(format!("Read-only: {err:#}")),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn editability_hint(&self) -> Option<&str> {
        match &self.editability {
            Editability::ReadOnly(reason) => Some(reason),
            Editability::Editable { .. } => Some("Double-click a cell to edit"),
            Editability::Unknown => None,
        }
    }

    pub(super) fn pending_edits(&self, cx: &App) -> usize {
        self.table.as_ref().map_or(0, |t| t.read(cx).delegate().pending())
    }

    fn is_editable(&self) -> bool {
        matches!(self.editability, Editability::Editable { .. })
    }

    /// Appends an empty row to fill in; it is inserted on Submit.
    pub(super) fn add_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(table) = self.table.clone().filter(|_| self.is_editable()) else {
            return;
        };
        let row = table.update(cx, |table, cx| {
            let row = table.delegate_mut().add_row();
            table.refresh(cx);
            cx.notify();
            row
        });
        self.selected_row = Some(row);
        // Start with the first column so typing a value is one step away.
        self.edit_cell(row, 0, window, cx);
    }

    /// Marks the selected row for deletion (or unmarks it).
    pub(super) fn delete_selected_row(&mut self, cx: &mut Context<Self>) {
        let (Some(table), Some(row)) = (
            self.table.clone().filter(|_| self.is_editable()),
            self.selected_row,
        ) else {
            return;
        };
        table.update(cx, |table, cx| {
            table.delegate_mut().toggle_delete(row);
            cx.notify();
        });
        cx.notify();
    }

    /// Opens the value editor for a cell (double-click).
    pub(super) fn edit_cell(&mut self, row: usize, col: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Editability::ReadOnly(reason) = &self.editability {
            return window.push_notification(Notification::info(reason.clone()), cx);
        }
        let Some(table) = self.table.clone() else { return };
        let (column, current) = {
            let delegate = table.read(cx).delegate();
            let original = delegate
                .set
                .rows
                .get(row)
                .and_then(|r| r.get(col))
                .cloned()
                .flatten();
            let current = delegate.edits.get(&(row, col)).cloned().unwrap_or(original);
            (
                delegate.set.columns.get(col).cloned().unwrap_or_default(),
                current,
            )
        };
        let is_null = cx.new(|_| current.is_none());
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(4)
                .default_value(current.unwrap_or_default())
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        window.open_dialog(cx, move |dialog, _, cx| {
            let (table, input, is_null) = (table.clone(), input.clone(), is_null.clone());
            let null_toggle = is_null.clone();
            dialog
                .w(px(480.))
                .title(format!("Edit {column}"))
                .child(
                    v_flex().gap_2().child(Textarea::new(&input)).child(
                        Checkbox::new("cell-null")
                            .label("NULL")
                            .checked(*is_null.read(cx))
                            .on_click(move |checked: &bool, _, cx| {
                                null_toggle.update(cx, |n, cx| {
                                    *n = *checked;
                                    cx.notify();
                                });
                            }),
                    ),
                )
                .footer(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .justify_end()
                        .child(
                            Button::new("cell-cancel")
                                .ghost()
                                .small()
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(Button::new("cell-ok").primary().small().label("OK").on_click(
                            move |_, window, cx| {
                                let value = (!*is_null.read(cx)).then(|| input.read(cx).value().to_string());
                                table.update(cx, |table, cx| {
                                    let delegate = table.delegate_mut();
                                    let original = delegate
                                        .set
                                        .rows
                                        .get(row)
                                        .and_then(|r| r.get(col))
                                        .cloned()
                                        .flatten();
                                    // Editing back to the original value drops the pending change.
                                    if value == original {
                                        delegate.edits.remove(&(row, col));
                                    } else {
                                        delegate.edits.insert((row, col), value);
                                    }
                                    cx.notify();
                                });
                                window.close_dialog(cx);
                            },
                        )),
                )
        });
    }

    pub(super) fn revert_edits(&mut self, cx: &mut Context<Self>) {
        if let Some(table) = &self.table {
            table.update(cx, |table, cx| {
                table.delegate_mut().revert();
                table.refresh(cx);
                cx.notify();
            });
        }
        cx.notify();
    }

    /// Runs the pending DELETE / UPDATE / INSERT statements, then reloads the results.
    pub(super) fn submit_edits(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Editability::Editable { table: target, key }, Some(table)) = (&self.editability, &self.table)
        else {
            return;
        };
        let statements = table.read(cx).delegate().statements(self.kind, target, key);
        let db = self.db.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let mut changed = 0;
                    for statement in &statements {
                        changed += db.execute(statement, 0)?.rows_affected;
                    }
                    anyhow::Ok(changed)
                })
                .await;
            this.update_in(cx, |this, window, cx| match result {
                Ok(changed) => {
                    let text = format!(
                        "Saved: {changed} row{} changed",
                        if changed == 1 { "" } else { "s" }
                    );
                    window.push_notification(Notification::success(text), cx);
                    this.rerun_last(window, cx);
                }
                Err(err) => window.push_notification(
                    Notification::error(format!("{err:#}")).title("Submit failed; edits are kept"),
                    cx,
                ),
            })
            .ok();
        })
        .detach();
    }

    /// Saves the results as CSV; read-only queries are re-run to export every row.
    pub(super) fn export_csv(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(set) = self.table.as_ref().map(|t| t.read(cx).delegate().set.clone()) else {
            return;
        };
        let full_query = (set.truncated && is_read_only(&self.last_query)).then(|| self.last_query.clone());
        let name = editable_table(&self.last_query).map_or_else(|| "result".to_string(), |t| t.name);
        let directory = dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("/"));
        let path = cx.prompt_for_new_path(&directory, Some(&format!("{name}.csv")));
        let db = self.db.clone();
        cx.spawn_in(window, async move |_, cx| {
            let Ok(Ok(Some(path))) = path.await else { return };
            let target = path.clone();
            let written = cx
                .background_spawn(async move {
                    let set = match full_query {
                        Some(query) => db
                            .execute(&query, EXPORT_ROW_LIMIT)?
                            .sets
                            .into_iter()
                            .rev()
                            .find(|s| !s.columns.is_empty())
                            .unwrap_or(set),
                        None => set,
                    };
                    std::fs::write(&target, db::to_csv(&set))?;
                    anyhow::Ok(set.rows.len())
                })
                .await;
            cx.update(|window, cx| {
                let notification = match written {
                    Ok(rows) => Notification::success(format!("Exported {rows} rows to {}", path.display())),
                    Err(err) => Notification::error(format!("{err:#}")).title("Export failed"),
                };
                window.push_notification(notification, cx);
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn render_edit_toolbar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.is_editable() {
            return None;
        }
        let pending = self.pending_edits(cx);
        let theme = cx.theme();
        let row_buttons = h_flex()
            .child(
                Button::new("add-row")
                    .ghost()
                    .xsmall()
                    .label("+ Row")
                    .tooltip("Add a row (inserted on Submit)")
                    .on_click(cx.listener(|this, _, window, cx| this.add_row(window, cx))),
            )
            .child(
                Button::new("delete-row")
                    .ghost()
                    .xsmall()
                    .label("− Row")
                    .tooltip("Delete the selected row (on Submit); click again to undo")
                    .on_click(cx.listener(|this, _, _, cx| this.delete_selected_row(cx))),
            );
        let pending_controls = (pending > 0).then(|| {
            h_flex()
                .gap_1()
                .child(
                    div()
                        .px_1p5()
                        .rounded_sm()
                        .text_color(theme.warning)
                        .bg(theme.warning.opacity(0.12))
                        .child(format!("{pending} pending")),
                )
                .child(
                    Button::new("submit-edits")
                        .primary()
                        .xsmall()
                        .label("Submit")
                        .tooltip("Write the changes to the database")
                        .on_click(cx.listener(|this, _, window, cx| this.submit_edits(window, cx))),
                )
                .child(
                    Button::new("revert-edits")
                        .ghost()
                        .xsmall()
                        .label("Revert")
                        .on_click(cx.listener(|this, _, _, cx| this.revert_edits(cx))),
                )
        });
        Some(
            h_flex()
                .gap_2()
                .child(row_buttons)
                .children(pending_controls)
                .into_any_element(),
        )
    }
}
