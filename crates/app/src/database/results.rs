//! Results grid backed by gpui-component's virtualized table.

use std::collections::BTreeMap;

use db::ResultSet;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::table::{Column, TableDelegate, TableState};
use gpui_kit::*;

/// Shows one result set; NULLs are dimmed like in JetBrains, pending edits highlighted.
pub struct ResultsDelegate {
    pub set: ResultSet,
    /// Edited but not yet submitted cells: (row, column) → new value (`None` = NULL).
    pub edits: BTreeMap<(usize, usize), Option<String>>,
}

impl ResultsDelegate {
    pub fn new(set: ResultSet) -> Self {
        Self {
            set,
            edits: BTreeMap::new(),
        }
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
        let base = if edited.is_some() {
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
