//! Database tool window: data sources, schema tree, consoles.

use db::{ConnectionConfig, DbKind, NodeKind, SchemaNode};
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenu};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::Workspace;
use crate::actions::{DatabaseAction, DatabaseOp};
use crate::database::{self, ConnState, QueryConsole};
use crate::editor::CenterTab;
use crate::style::{self, lucide};

/// `SELECT * …` for a table, quoted for the database's dialect.
pub fn select_table_sql(kind: DbKind, schema: &str, table: &str) -> String {
    let quote = |id: &str| match kind {
        DbKind::MySql => format!("`{}`", id.replace('`', "``")),
        _ => format!("\"{}\"", id.replace('"', "\"\"")),
    };
    let qualified = match kind {
        DbKind::Postgres if schema != "public" => format!("{}.{}", quote(schema), quote(table)),
        _ => quote(table),
    };
    format!("SELECT * FROM {qualified} LIMIT 100;")
}

/// A command that shows a Redis key's value, by key type.
pub fn redis_inspect_command(key: &str, key_type: &str) -> String {
    let key = format!("\"{}\"", key.replace('\\', "\\\\").replace('"', "\\\""));
    match key_type {
        "hash" => format!("HGETALL {key}"),
        "list" => format!("LRANGE {key} 0 99"),
        "set" => format!("SMEMBERS {key}"),
        "zset" => format!("ZRANGE {key} 0 99 WITHSCORES"),
        "stream" => format!("XRANGE {key} - + COUNT 100"),
        _ => format!("GET {key}"),
    }
}

impl Workspace {
    pub(crate) fn save_data_source(
        &mut self,
        config: ConnectionConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = config.id.clone();
        if let Err(err) = self.databases.upsert(config) {
            return self.notify_error(format!("{err:#}"), window, cx);
        }
        self.databases.expanded.insert(id.clone());
        self.connect_data_source(&id, cx);
    }

    fn connect_data_source(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(config) = self.databases.config(id).cloned() else {
            return;
        };
        let id = id.to_string();
        self.databases.states.insert(id.clone(), ConnState::Connecting);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { database::connect(&config) })
                .await;
            this.update(cx, |ws, cx| {
                let state = match result {
                    Ok((db, schema)) => ConnState::Connected { db, schema },
                    Err(err) => ConnState::Failed(format!("{err:#}")),
                };
                ws.databases.states.insert(id, state);
                ws.open_pending_console(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Opens a console on `id` with `sql`, connecting first if needed.
    fn open_console(
        &mut self,
        id: &str,
        sql: String,
        run: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(config), Some(db)) = (self.databases.config(id).cloned(), self.databases.connected(id))
        else {
            self.pending_console = Some((id.to_string(), sql, run));
            if !matches!(self.databases.states.get(id), Some(ConnState::Connecting)) {
                self.connect_data_source(id, cx);
            }
            return;
        };
        let schema = self.databases.schema(id);
        let console = cx.new(|cx| QueryConsole::new(&config, db, schema, sql, window, cx));
        if run {
            console.update(cx, |console, cx| console.run(window, cx));
        }
        self.tabs.push(CenterTab::Console {
            title: format!("{} console", config.name),
            console,
        });
        let ix = self.tabs.len() - 1;
        self.activate_tab(ix, window, cx);
    }

    fn open_pending_console(&mut self, cx: &mut Context<Self>) {
        let Some((id, sql, run)) = self.pending_console.take() else {
            return;
        };
        match self.databases.states.get(&id) {
            Some(ConnState::Connected { .. }) => {
                let workspace = cx.entity().downgrade();
                cx.defer(move |cx| {
                    let Some(window) = cx.active_window() else { return };
                    window
                        .update(cx, |_, window, cx| {
                            workspace
                                .update(cx, |ws, cx| ws.open_console(&id, sql, run, window, cx))
                                .ok();
                        })
                        .ok();
                });
            }
            Some(ConnState::Connecting) => self.pending_console = Some((id, sql, run)),
            _ => {}
        }
    }

    pub(super) fn handle_database_action(
        &mut self,
        action: &DatabaseAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = action.id.to_string();
        match action.op {
            DatabaseOp::NewDataSource => {
                database::open_connection_dialog(None, cx.entity().downgrade(), window, cx);
            }
            DatabaseOp::OpenConsole => self.open_console(&id, String::new(), false, window, cx),
            DatabaseOp::Edit => {
                let config = self.databases.config(&id).cloned();
                database::open_connection_dialog(config, cx.entity().downgrade(), window, cx);
            }
            DatabaseOp::Refresh => self.connect_data_source(&id, cx),
            DatabaseOp::Disconnect => {
                self.databases.states.remove(&id);
                cx.notify();
            }
            DatabaseOp::Remove => {
                let name = self.databases.config(&id).map_or_default(|c| c.name.clone());
                let workspace = cx.entity().downgrade();
                window.open_alert_dialog(cx, move |dialog, _, cx| {
                    let (workspace, id) = (workspace.clone(), id.clone());
                    style::danger_confirm(dialog, "trash", "Remove", cx)
                        .title(format!("Remove data source {name}?"))
                        .child("Its saved password is deleted from the Keychain too.")
                        .on_ok(move |_, window, cx| {
                            workspace
                                .update(cx, |ws, cx| {
                                    if let Err(err) = ws.databases.remove(&id) {
                                        window.push_notification(Notification::error(format!("{err:#}")), cx);
                                    }
                                    ws.close_consoles(&id, window, cx);
                                    cx.notify();
                                })
                                .ok();
                            true
                        })
                });
            }
        }
    }

    /// Closes every console tab of the data source `id`.
    fn close_consoles(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        for ix in (0..self.tabs.len()).rev() {
            if let Some(CenterTab::Console { console, .. }) = self.tabs.get(ix)
                && console.read(cx).connection_id == id
            {
                self.close_tab_now(ix, window, cx);
            }
        }
    }

    fn data_source_menu(&self, menu: PopupMenu, id: &str) -> PopupMenu {
        let action = |op| DatabaseAction::boxed(op, id.to_string());
        menu.action_context(self.focus_handle.clone())
            .menu("Open Console", action(DatabaseOp::OpenConsole))
            .menu("Refresh", action(DatabaseOp::Refresh))
            .menu("Disconnect", action(DatabaseOp::Disconnect))
            .separator()
            .menu("Edit…", action(DatabaseOp::Edit))
            .menu("Remove…", action(DatabaseOp::Remove))
    }

    fn toggle_db_node(&mut self, key: String, cx: &mut Context<Self>) {
        if !self.databases.expanded.remove(&key) {
            self.databases.expanded.insert(key);
        }
        cx.notify();
    }

    pub(super) fn render_database_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut rows: Vec<AnyElement> = Vec::new();
        for config in &self.databases.configs {
            let expanded = self.databases.expanded.contains(&config.id);
            let state = self.databases.states.get(&config.id);
            rows.push(self.render_data_source_row(config, expanded, state, cx));
            if !expanded {
                continue;
            }
            match state {
                Some(ConnState::Connected { schema, .. }) => {
                    for node in schema {
                        self.render_schema_node(config, node, &config.id, "", 1, &mut rows, cx);
                    }
                    if schema.is_empty() {
                        rows.push(self.render_hint("No objects", 1, cx));
                    }
                }
                Some(ConnState::Failed(err)) => rows.push(self.render_hint(err, 1, cx)),
                Some(ConnState::Connecting) => rows.push(self.render_hint("Connecting…", 1, cx)),
                None => {}
            }
        }

        let empty = self.databases.configs.is_empty();
        let theme = cx.theme();
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .px_2()
                    .py_1()
                    .gap_1()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        style::icon_button_sized(
                            "db-new",
                            IconName::Plus,
                            "New Data Source",
                            style::BUTTON_SMALL,
                        )
                        .on_click(cx.listener(|_, _, window, cx| {
                            database::open_connection_dialog(None, cx.entity().downgrade(), window, cx);
                        })),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child("Data sources"),
                    ),
            )
            .child(
                div()
                    .id("db-tree")
                    .flex_1()
                    .overflow_y_scroll()
                    .py_1()
                    .children(rows)
                    .when(empty, |el| {
                        el.child(
                            v_flex()
                                .p_4()
                                .gap_2()
                                .items_center()
                                .text_sm()
                                .text_color(theme.muted_foreground)
                                .child(lucide("database").size_6())
                                .child("No data sources yet")
                                .child(
                                    component::button::Button::new("db-empty-new")
                                        .small()
                                        .label("Add Data Source")
                                        .on_click(cx.listener(|_, _, window, cx| {
                                            database::open_connection_dialog(
                                                None,
                                                cx.entity().downgrade(),
                                                window,
                                                cx,
                                            );
                                        })),
                                ),
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_hint(&self, text: &str, depth: usize, cx: &App) -> AnyElement {
        div()
            .pl(px(12. + 16. * depth as f32))
            .pr_2()
            .py_0p5()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(text.to_string())
            .into_any_element()
    }

    fn row_base(id: impl Into<ElementId>, depth: usize, cx: &App) -> Stateful<Div> {
        h_flex()
            .id(id)
            .h(px(24.))
            .pl(px(8. + 16. * depth as f32))
            .pr_2()
            .gap_1p5()
            .text_sm()
            .cursor_pointer()
            .hover(|s| s.bg(cx.theme().list_hover))
    }

    fn chevron(expanded: bool, cx: &App) -> Div {
        div().flex_none().w(style::TREE_CHEVRON).child(
            Icon::new(if expanded {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            })
            .with_size(style::TREE_CHEVRON)
            .text_color(cx.theme().muted_foreground),
        )
    }

    fn render_data_source_row(
        &self,
        config: &ConnectionConfig,
        expanded: bool,
        state: Option<&ConnState>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let tint = match state {
            Some(ConnState::Connected { .. }) => theme.green,
            Some(ConnState::Failed(_)) => theme.danger,
            Some(ConnState::Connecting) => theme.yellow,
            None => theme.muted_foreground,
        };
        let id = config.id.clone();
        let menu_id = config.id.clone();
        let workspace = cx.entity().downgrade();
        Self::row_base(SharedString::from(format!("ds-{}", config.id)), 0, cx)
            .child(Self::chevron(expanded, cx))
            .child(lucide("database").with_size(style::TREE_ICON).text_color(tint))
            .child(div().flex_none().child(config.name.clone()))
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(config.summary()),
            )
            .on_click(cx.listener(move |ws, _, _, cx| {
                let opening = !ws.databases.expanded.contains(&id);
                ws.toggle_db_node(id.clone(), cx);
                if opening && !ws.databases.states.contains_key(&id) {
                    ws.connect_data_source(&id, cx);
                }
            }))
            .context_menu(move |menu, _, cx| match workspace.upgrade() {
                Some(ws) => ws.read(cx).data_source_menu(menu, &menu_id),
                None => menu,
            })
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_schema_node(
        &self,
        config: &ConnectionConfig,
        node: &SchemaNode,
        parent_key: &str,
        schema_name: &str,
        depth: usize,
        rows: &mut Vec<AnyElement>,
        cx: &mut Context<Self>,
    ) {
        let theme = cx.theme();
        let key = format!("{parent_key}/{}", node.name);
        let expanded = self.databases.expanded.contains(&key);
        let has_children = !node.children.is_empty();
        let (icon, tint, detail) = match &node.kind {
            NodeKind::Schema => ("layers", theme.blue, None),
            NodeKind::Table => ("table", theme.cyan, None),
            NodeKind::View => ("eye", theme.magenta, None),
            NodeKind::Column { data_type } => ("columns-3", theme.muted_foreground, Some(data_type.clone())),
            NodeKind::Key { key_type } => ("key-round", theme.yellow, Some(key_type.clone())),
        };

        let (id, kind) = (config.id.clone(), config.kind);
        let (name, schema) = (
            node.name.clone(),
            if matches!(node.kind, NodeKind::Schema) {
                node.name.clone()
            } else {
                schema_name.to_string()
            },
        );
        let node_kind = node.kind.clone();
        let toggle_key = key.clone();
        let row = Self::row_base(SharedString::from(key.clone()), depth, cx)
            .child(if has_children {
                Self::chevron(expanded, cx)
            } else {
                div().flex_none().w(style::TREE_CHEVRON)
            })
            .child(lucide(icon).with_size(style::TREE_ICON).text_color(tint))
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .child(node.name.clone()),
            )
            .children(detail.map(|d| {
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(d)
            }))
            .on_click(
                cx.listener(move |ws, event: &ClickEvent, window, cx| match &node_kind {
                    NodeKind::Table | NodeKind::View if event.click_count() >= 2 => {
                        ws.open_console(&id, select_table_sql(kind, &schema, &name), true, window, cx);
                    }
                    NodeKind::Key { key_type } => {
                        ws.open_console(&id, redis_inspect_command(&name, key_type), true, window, cx);
                    }
                    _ if has_children => ws.toggle_db_node(toggle_key.clone(), cx),
                    _ => {}
                }),
            );
        rows.push(row.into_any_element());

        if expanded {
            let schema = if matches!(node.kind, NodeKind::Schema) {
                node.name.as_str()
            } else {
                schema_name
            };
            for child in &node.children {
                self.render_schema_node(config, child, &key, schema, depth + 1, rows, cx);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use db::DbKind;

    use super::{redis_inspect_command, select_table_sql};

    #[test]
    fn quotes_table_selects_per_dialect() {
        assert_eq!(
            select_table_sql(DbKind::Postgres, "public", "users"),
            "SELECT * FROM \"users\" LIMIT 100;"
        );
        assert_eq!(
            select_table_sql(DbKind::Postgres, "billing", "in\"voices"),
            "SELECT * FROM \"billing\".\"in\"\"voices\" LIMIT 100;"
        );
        assert_eq!(
            select_table_sql(DbKind::MySql, "app", "order"),
            "SELECT * FROM `order` LIMIT 100;"
        );
    }

    #[test]
    fn inspects_redis_keys_by_type() {
        assert_eq!(redis_inspect_command("user:1", "hash"), "HGETALL \"user:1\"");
        assert_eq!(redis_inspect_command("q", "list"), "LRANGE \"q\" 0 99");
        assert_eq!(redis_inspect_command("a\"b", "string"), "GET \"a\\\"b\"");
    }
}
