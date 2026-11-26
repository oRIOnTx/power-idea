//! Query console tab: an editor for SQL / Redis commands and a results grid.
//! Cmd+Enter runs the selection, or the whole console when nothing is selected.
//! Editing and export of the results live in `editing.rs`.

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use db::{ConnectionConfig, Database, DbKind, QueryOutcome, SchemaNode};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Editor, EditorState, TabSize};
use gpui_kit::component::table::{DataTable, TableEvent, TableState};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, h_flex, resizable_panel, v_flex, v_resizable,
};
use gpui_kit::*;

use super::completion::SqlCompletion;
use super::editing::Editability;
use super::results::ResultsDelegate;
use crate::style;

pub const CONSOLE_CONTEXT: &str = "QueryConsole";

actions!(console, [RunQuery]);

enum Status {
    Idle,
    Running,
    Done(QueryOutcome),
    Failed(String),
}

pub struct QueryConsole {
    pub connection_id: String,
    pub title: String,
    pub(super) kind: DbKind,
    pub(super) db: Arc<Database>,
    editor: Entity<EditorState>,
    pub(super) table: Option<Entity<TableState<ResultsDelegate>>>,
    /// The query that produced the results (for edits, export and refresh).
    pub(super) last_query: String,
    pub(super) editability: Editability,
    /// Row of the last selected cell (for "− Row").
    pub(super) selected_row: Option<usize>,
    _table_events: Option<Subscription>,
    status: Status,
    run_task: Task<()>,
    focus_handle: FocusHandle,
}

impl QueryConsole {
    pub fn new(
        config: &ConnectionConfig,
        db: Arc<Database>,
        schema: Vec<SchemaNode>,
        initial: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let language = if config.kind.is_sql() { "sql" } else { "" };
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(language)
                .tab_size(TabSize {
                    tab_size: 2,
                    hard_tabs: false,
                })
                .soft_wrap(false)
                .default_value(initial)
        });
        if config.kind.is_sql() {
            editor.update(cx, |editor, _| {
                editor.lsp_mut().completion_provider = Some(Rc::new(SqlCompletion { schema }));
            });
        }
        Self {
            connection_id: config.id.clone(),
            title: config.name.clone(),
            kind: config.kind,
            db,
            editor,
            table: None,
            last_query: String::new(),
            editability: Editability::Unknown,
            selected_row: None,
            _table_events: None,
            status: Status::Idle,
            run_task: Task::ready(()),
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.editor.update(cx, |editor, cx| editor.focus(window, cx));
    }

    /// Runs the selection, or the whole console.
    pub fn run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let editor = self.editor.read(cx);
        let selected = editor.selected_value();
        let query = if selected.trim().is_empty() {
            editor.value()
        } else {
            selected
        }
        .to_string();
        self.execute(query, window, cx);
    }

    /// Runs the last query again (after submitting edits).
    pub(super) fn rerun_last(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.execute(self.last_query.clone(), window, cx);
    }

    // reason: `_`-prefixed fields keep a task/subscription alive; reassigning them is the intent.
    #[allow(clippy::used_underscore_binding)]
    fn execute(&mut self, query: String, window: &mut Window, cx: &mut Context<Self>) {
        if query.trim().is_empty() {
            return;
        }
        self.last_query.clone_from(&query);
        self.status = Status::Running;
        cx.notify();

        let db = self.db.clone();
        self.run_task = cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move { db.execute(&query, db::DEFAULT_ROW_LIMIT) })
                .await;
            this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(outcome) => {
                        // Show the last result set that has columns (e.g. the final SELECT).
                        let set = outcome.sets.iter().rev().find(|s| !s.columns.is_empty()).cloned();
                        this.table = set.map(|set| {
                            cx.new(|cx| {
                                TableState::new(ResultsDelegate::new(set), window, cx)
                                    .col_resizable(true)
                                    .cell_selectable(true)
                            })
                        });
                        this.selected_row = None;
                        this._table_events = this.table.as_ref().map(|table| {
                            cx.subscribe_in(table, window, |this, _, event: &TableEvent, window, cx| {
                                match event {
                                    TableEvent::DoubleClickedCell(row, col) => {
                                        this.edit_cell(*row, *col, window, cx);
                                    }
                                    TableEvent::SelectCell(row, _) | TableEvent::SelectRow(row) => {
                                        this.selected_row = Some(*row);
                                    }
                                    _ => {}
                                }
                            })
                        });
                        this.status = Status::Done(outcome);
                        this.detect_editability(cx);
                    }
                    Err(err) => this.status = Status::Failed(format!("{err:#}")),
                }
                cx.notify();
            })
            .ok();
        });
    }

    fn render_status(&self, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let ms = |d: Duration| format!("{} ms", d.as_millis());
        match &self.status {
            Status::Idle => div()
                .text_color(theme.muted_foreground)
                .child(if self.kind.is_sql() {
                    "⌘↩ to run"
                } else {
                    "⌘↩ to run · one command per line"
                })
                .into_any_element(),
            Status::Running => div()
                .text_color(theme.primary)
                .child("Running…")
                .into_any_element(),
            Status::Failed(_) => div().text_color(theme.danger).child("Failed").into_any_element(),
            Status::Done(outcome) => {
                let rows = self.table.as_ref().map(|t| t.read(cx).delegate().set.rows.len());
                let truncated = self
                    .table
                    .as_ref()
                    .is_some_and(|t| t.read(cx).delegate().set.truncated);
                let summary = match rows {
                    Some(n) if truncated => format!("first {n} rows"),
                    Some(n) => format!("{n} row{}", if n == 1 { "" } else { "s" }),
                    None => format!("{} affected", outcome.rows_affected),
                };
                h_flex()
                    .gap_2()
                    .child(style::signal_dot(theme.green))
                    .child(format!("{summary} · {}", ms(outcome.elapsed)))
                    .into_any_element()
            }
        }
    }

    fn render_results(&self, cx: &App) -> AnyElement {
        let theme = cx.theme();
        match (&self.status, &self.table) {
            (Status::Failed(message), _) => div()
                .id("query-error")
                .size_full()
                .p_3()
                .overflow_y_scroll()
                .font_family(theme.mono_font_family.clone())
                .text_sm()
                .text_color(theme.danger)
                .child(message.clone())
                .into_any_element(),
            (_, Some(table)) => DataTable::new(table).small().into_any_element(),
            (Status::Done(outcome), None) => div()
                .p_3()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(format!(
                    "Statement executed · {} rows affected",
                    outcome.rows_affected
                ))
                .into_any_element(),
            _ => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child("Results appear here")
                .into_any_element(),
        }
    }
}

impl Focusable for QueryConsole {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for QueryConsole {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let edit_toolbar = self.render_edit_toolbar(cx);
        let theme = cx.theme();
        let running = matches!(self.status, Status::Running);
        let toolbar = h_flex()
            .h(px(36.))
            .px_2()
            .gap_2()
            .border_b_1()
            .border_color(theme.border)
            .bg(theme.tab_bar)
            .text_xs()
            .child(
                Button::new("run-query")
                    .primary()
                    .small()
                    .icon(Icon::empty().path("icons/play.svg"))
                    .label("Run")
                    .tooltip("Run (⌘↩)")
                    .disabled(running)
                    .on_click(cx.listener(|this, _, window, cx| this.run(window, cx))),
            )
            .child(
                h_flex()
                    .gap_1p5()
                    .px_2()
                    .py_0p5()
                    .rounded(theme.radius)
                    .bg(theme.muted)
                    .child(Icon::empty().path("icons/database.svg").size_3p5())
                    .child(self.title.clone())
                    .child(div().text_color(theme.muted_foreground).child(self.kind.label())),
            )
            .child(div().flex_1())
            .children(edit_toolbar)
            .children(
                self.editability_hint()
                    .map(|hint| div().text_color(theme.muted_foreground).child(hint.to_string())),
            )
            .child(
                Button::new("export-csv")
                    .ghost()
                    .xsmall()
                    .label("Export CSV")
                    .tooltip("Save the results as CSV")
                    .disabled(self.table.is_none())
                    .on_click(cx.listener(|this, _, window, cx| this.export_csv(window, cx))),
            )
            .child(self.render_status(cx));

        v_flex()
            .key_context(CONSOLE_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &RunQuery, window, cx| this.run(window, cx)))
            .size_full()
            .child(toolbar)
            .child(
                v_resizable("console")
                    .child(
                        resizable_panel().size(px(220.)).child(
                            Editor::new(&self.editor)
                                .bordered(false)
                                .size_full()
                                .font_features(FontFeatures::disable_ligatures()),
                        ),
                    )
                    .child(
                        resizable_panel().child(
                            div()
                                .size_full()
                                .border_t_1()
                                .border_color(theme.border)
                                .child(self.render_results(cx)),
                        ),
                    ),
            )
    }
}
