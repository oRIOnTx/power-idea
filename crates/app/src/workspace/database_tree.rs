//! Database tool window tree: data sources, their databases (Discover Databases), schemas,
//! tables, columns and Redis keys.

use db::{ConnectionConfig, NodeKind, SchemaNode};
use gpui_kit::component::menu::ContextMenuExt as _;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::Workspace;
use super::database_panel::{redis_inspect_command, select_table_sql};
use crate::database::{self, ConnState};
use crate::style::{self, lucide};

impl Workspace {
    pub(super) fn toggle_db_node(&mut self, key: String, cx: &mut Context<Self>) {
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
                Some(ConnState::Connected { .. }) if !config.databases.is_empty() => {
                    for name in &config.databases {
                        self.render_database_node(config, name, &mut rows, cx);
                    }
                }
                _ => self.render_connection_children(config, None, state, &config.id, 1, &mut rows, cx),
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

    /// The schema of a connection, or its connecting / error state.
    #[allow(clippy::too_many_arguments)]
    fn render_connection_children(
        &self,
        config: &ConnectionConfig,
        database: Option<&str>,
        state: Option<&ConnState>,
        parent_key: &str,
        depth: usize,
        rows: &mut Vec<AnyElement>,
        cx: &mut Context<Self>,
    ) {
        match state {
            Some(ConnState::Connected { schema, .. }) => {
                for node in schema {
                    self.render_schema_node(config, database, node, parent_key, "", depth, rows, cx);
                }
                if schema.is_empty() {
                    rows.push(self.render_hint("No objects", depth, cx));
                }
            }
            Some(ConnState::Failed(err)) => rows.push(self.render_hint(err, depth, cx)),
            Some(ConnState::Connecting) => rows.push(self.render_hint("Connecting…", depth, cx)),
            None => {}
        }
    }

    /// A database picked with Discover Databases; it connects when first expanded.
    fn render_database_node(
        &self,
        config: &ConnectionConfig,
        name: &str,
        rows: &mut Vec<AnyElement>,
        cx: &mut Context<Self>,
    ) {
        let key = format!("{}/db:{name}", config.id);
        let expanded = self.databases.expanded.contains(&key);
        let state = self.databases.state(&config.id, Some(name));
        let (id, database, toggle_key) = (config.id.clone(), name.to_string(), key.clone());
        let (menu_id, menu_database) = (config.id.clone(), name.to_string());
        let workspace = cx.entity().downgrade();
        let row = Self::row_base(SharedString::from(key.clone()), 1, cx)
            .child(Self::chevron(expanded, cx))
            .child(
                lucide("database")
                    .with_size(style::TREE_ICON)
                    .text_color(Self::state_tint(state, cx)),
            )
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .child(name.to_string()),
            )
            .on_click(cx.listener(move |ws, _, _, cx| {
                let opening = !ws.databases.expanded.contains(&toggle_key);
                ws.toggle_db_node(toggle_key.clone(), cx);
                if opening && ws.databases.state(&id, Some(&database)).is_none() {
                    ws.connect_target(&id, Some(&database), cx);
                }
            }))
            .context_menu(move |menu, _, cx| match workspace.upgrade() {
                Some(ws) => ws.read(cx).database_menu(menu, &menu_id, &menu_database),
                None => menu,
            });
        rows.push(row.into_any_element());
        if expanded {
            self.render_connection_children(config, Some(name), state, &key, 2, rows, cx);
        }
    }

    fn state_tint(state: Option<&ConnState>, cx: &App) -> Hsla {
        let theme = cx.theme();
        match state {
            Some(ConnState::Connected { .. }) => theme.green,
            Some(ConnState::Failed(_)) => theme.danger,
            Some(ConnState::Connecting) => theme.yellow,
            None => theme.muted_foreground,
        }
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
        let tint = Self::state_tint(state, cx);
        let theme = cx.theme();
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
                if opening && ws.databases.state(&id, None).is_none() {
                    ws.connect_target(&id, None, cx);
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
        database: Option<&str>,
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

        let (id, kind, target) = (config.id.clone(), config.kind, database.map(str::to_string));
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
                        let sql = select_table_sql(kind, &schema, &name);
                        ws.open_console(&id, target.as_deref(), sql, true, window, cx);
                    }
                    NodeKind::Key { key_type } => {
                        let command = redis_inspect_command(&name, key_type);
                        ws.open_console(&id, None, command, true, window, cx);
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
                self.render_schema_node(config, database, child, &key, schema, depth + 1, rows, cx);
            }
        }
    }
}
