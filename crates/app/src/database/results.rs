//! Results grid backed by gpui-component's virtualized table.

use db::ResultSet;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::table::{Column, TableDelegate, TableState};
use gpui_kit::*;

/// Shows one result set; NULLs are dimmed like in JetBrains.
pub struct ResultsDelegate {
    pub set: ResultSet,
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
        let cell = self
            .set
            .rows
            .get(row_ix)
            .and_then(|row| row.get(col_ix))
            .cloned()
            .flatten();
        let theme = cx.theme();
        let base = div()
            .w_full()
            .overflow_hidden()
            .text_ellipsis()
            .whitespace_nowrap();
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
