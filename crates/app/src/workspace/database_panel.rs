//! Database tool window: data source actions, connections and consoles (the tree is in
//! `database_tree.rs`).

use db::{ConnectionConfig, DbKind};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;

use super::Workspace;
use crate::actions::{DatabaseAction, DatabaseOp};
use crate::database::{self, ConnState, QueryConsole};
use crate::editor::CenterTab;
use crate::style;

/// A console to open once its connection is ready.
pub(super) struct PendingConsole {
    id: String,
    /// One of the data source's databases (Discover Databases); `None`: its own connection.
    database: Option<String>,
    sql: String,
    run: bool,
}

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
        self.connect_target(&id, None, cx);
    }

    /// Connects the data source `id`, or one of its databases.
    pub(super) fn connect_target(&mut self, id: &str, database: Option<&str>, cx: &mut Context<Self>) {
        let Some(config) = self.databases.config(id) else {
            return;
        };
        let config = database.map_or_else(|| config.clone(), |name| config.for_database(name));
        let (id, database) = (id.to_string(), database.map(str::to_string));
        self.databases
            .set_state(&id, database.as_deref(), ConnState::Connecting);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { database::connect(&config) })
                .await;
            this.update(cx, |ws, cx| {
                // Removed, or disconnected while connecting: the result is stale.
                if !matches!(
                    ws.databases.state(&id, database.as_deref()),
                    Some(ConnState::Connecting)
                ) {
                    return;
                }
                let state = match result {
                    Ok((db, schema)) => ConnState::Connected { db, schema },
                    Err(err) => ConnState::Failed(format!("{err:#}")),
                };
                ws.databases.set_state(&id, database.as_deref(), state);
                ws.open_pending_console(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Opens a console on `id` (or its `database`) with `sql`, connecting first if needed.
    pub(super) fn open_console(
        &mut self,
        id: &str,
        database: Option<&str>,
        sql: String,
        run: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(config), Some(db)) = (
            self.databases.config(id).cloned(),
            self.databases.connected(id, database),
        ) else {
            self.pending_console = Some(PendingConsole {
                id: id.to_string(),
                database: database.map(str::to_string),
                sql,
                run,
            });
            if !matches!(self.databases.state(id, database), Some(ConnState::Connecting)) {
                self.connect_target(id, database, cx);
            }
            return;
        };
        let schema = self.databases.schema(id, database);
        let title = match database {
            Some(name) if name != config.database => format!("{} · {name} console", config.name),
            _ => format!("{} console", config.name),
        };
        let console = cx.new(|cx| QueryConsole::new(&config, db, schema, sql, window, cx));
        if run {
            console.update(cx, |console, cx| console.run(window, cx));
        }
        self.tabs.push(CenterTab::Console { title, console });
        let ix = self.tabs.len() - 1;
        self.activate_tab(ix, window, cx);
    }

    fn open_pending_console(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_console.take() else {
            return;
        };
        match self.databases.state(&pending.id, pending.database.as_deref()) {
            Some(ConnState::Connected { .. }) => {
                let workspace = cx.entity().downgrade();
                cx.defer(move |cx| {
                    let Some(window) = cx.active_window() else { return };
                    let PendingConsole {
                        id,
                        database,
                        sql,
                        run,
                    } = pending;
                    window
                        .update(cx, |_, window, cx| {
                            workspace
                                .update(cx, |ws, cx| {
                                    ws.open_console(&id, database.as_deref(), sql, run, window, cx);
                                })
                                .ok();
                        })
                        .ok();
                });
            }
            Some(ConnState::Connecting) => self.pending_console = Some(pending),
            _ => {}
        }
    }

    /// Opens the Databases dialog and lists the server's databases into it, over the data
    /// source's own connection when it is open (connecting it first otherwise).
    fn discover_databases(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(config) = self.databases.config(id).cloned() else {
            return;
        };
        let picker = database::open_database_picker(&config, cx.entity().downgrade(), window, cx);
        let existing = self.databases.connected(id, None);
        let id = id.to_string();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let (db, fresh) = if let Some(db) = existing {
                        (db, None)
                    } else {
                        let (db, schema) = database::connect(&config)?;
                        (db.clone(), Some(ConnState::Connected { db, schema }))
                    };
                    anyhow::Ok((db.databases(), fresh))
                })
                .await;
            this.update(cx, |ws, cx| {
                let names = match result {
                    Ok((names, fresh)) => {
                        // Keep the connection discovery opened, unless one appeared meanwhile.
                        if let Some(state) = fresh
                            && ws.databases.config(&id).is_some()
                            && ws.databases.connected(&id, None).is_none()
                        {
                            ws.databases.set_state(&id, None, state);
                            cx.notify();
                        }
                        names
                    }
                    Err(err) => Err(err),
                };
                let names = names.map_err(|err| format!("{err:#}"));
                picker.update(cx, |picker, cx| picker.finish(names, cx));
            })
            .ok();
        })
        .detach();
    }

    /// Shows `databases` under the data source `id` (empty: its own database again).
    pub(crate) fn show_databases(
        &mut self,
        id: &str,
        databases: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(err) = self.databases.set_databases(id, databases) {
            return self.notify_error(format!("{err:#}"), window, cx);
        }
        self.databases.expanded.insert(id.to_string());
        if self.databases.state(id, None).is_none() {
            self.connect_target(id, None, cx);
        }
        cx.notify();
    }

    pub(super) fn handle_database_action(
        &mut self,
        action: &DatabaseAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = action.id.to_string();
        let database = (!action.database.is_empty()).then(|| action.database.to_string());
        match action.op {
            DatabaseOp::NewDataSource => {
                database::open_connection_dialog(None, cx.entity().downgrade(), window, cx);
            }
            DatabaseOp::OpenConsole => {
                self.open_console(&id, database.as_deref(), String::new(), false, window, cx);
            }
            DatabaseOp::Edit => {
                let config = self.databases.config(&id).cloned();
                database::open_connection_dialog(config, cx.entity().downgrade(), window, cx);
            }
            DatabaseOp::Refresh if database.is_some() => self.connect_target(&id, database.as_deref(), cx),
            DatabaseOp::Refresh => {
                self.databases.forget(&id);
                self.connect_target(&id, None, cx);
            }
            DatabaseOp::Disconnect => {
                self.databases.forget(&id);
                cx.notify();
            }
            DatabaseOp::DiscoverDatabases => self.discover_databases(&id, window, cx),
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

    pub(super) fn data_source_menu(&self, menu: PopupMenu, id: &str) -> PopupMenu {
        let action = |op| DatabaseAction::boxed(op, id.to_string());
        let discover = self
            .databases
            .config(id)
            .is_some_and(|c| c.kind.lists_databases());
        let menu = menu
            .action_context(self.focus_handle.clone())
            .menu("Open Console", action(DatabaseOp::OpenConsole))
            .menu("Refresh", action(DatabaseOp::Refresh))
            .menu("Disconnect", action(DatabaseOp::Disconnect));
        let menu = if discover {
            menu.separator()
                .menu("Discover Databases…", action(DatabaseOp::DiscoverDatabases))
        } else {
            menu
        };
        menu.separator()
            .menu("Edit…", action(DatabaseOp::Edit))
            .menu("Remove…", action(DatabaseOp::Remove))
    }

    /// The menu of one of a data source's databases.
    pub(super) fn database_menu(&self, menu: PopupMenu, id: &str, database: &str) -> PopupMenu {
        let action = |op| DatabaseAction::boxed_for(op, id.to_string(), database.to_string());
        menu.action_context(self.focus_handle.clone())
            .menu("Open Console", action(DatabaseOp::OpenConsole))
            .menu("Refresh", action(DatabaseOp::Refresh))
            .separator()
            .menu(
                "Discover Databases…",
                DatabaseAction::boxed(DatabaseOp::DiscoverDatabases, id.to_string()),
            )
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
