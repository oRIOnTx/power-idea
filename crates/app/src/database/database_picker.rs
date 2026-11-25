//! "Databases" dialog (Discover Databases): lists the databases the server lets the user connect
//! to and picks the ones the Database tool window shows under the data source.

use std::collections::BTreeSet;

use db::ConnectionConfig;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::*;

use crate::workspace::Workspace;

/// One row of the dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub name: String,
    /// `false` for a database shown before that the server no longer lists (dropped, or the
    /// privilege was revoked): kept so the user decides, never dropped silently.
    pub available: bool,
}

/// The server's databases plus the shown ones it no longer lists, sorted by name.
pub fn choices(available: &[String], shown: &[String]) -> Vec<Choice> {
    let mut choices: Vec<Choice> = available
        .iter()
        .map(|name| Choice {
            name: name.clone(),
            available: true,
        })
        .collect();
    for name in shown {
        if !available.contains(name) {
            choices.push(Choice {
                name: name.clone(),
                available: false,
            });
        }
    }
    choices.sort_by(|a, b| a.name.cmp(&b.name));
    choices
}

/// What starts checked: the databases already shown, else the data source's own database.
pub fn preselected(config: &ConnectionConfig) -> BTreeSet<String> {
    if config.databases.is_empty() {
        (!config.database.is_empty())
            .then(|| config.database.clone())
            .into_iter()
            .collect()
    } else {
        config.databases.iter().cloned().collect()
    }
}

/// Case-insensitive substring match of the search field.
pub fn matches(name: &str, query: &str) -> bool {
    name.to_lowercase().contains(&query.trim().to_lowercase())
}

enum Discovery {
    Loading,
    Failed(String),
    Loaded(Vec<Choice>),
}

pub struct DatabasePicker {
    id: String,
    /// `user@host:port`, for the empty and error messages.
    server: String,
    shown: Vec<String>,
    discovery: Discovery,
    selected: BTreeSet<String>,
    search: Entity<InputState>,
    workspace: WeakEntity<Workspace>,
    focused_once: bool,
}

/// Opens the dialog in its loading state; the caller reports the result with `finish`.
pub fn open_database_picker(
    config: &ConnectionConfig,
    workspace: WeakEntity<Workspace>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<DatabasePicker> {
    let picker = cx.new(|cx| DatabasePicker::new(config, workspace, window, cx));
    let title = format!("Databases of {}", config.name);
    let dialog_picker = picker.clone();
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .w(px(440.))
            .title(title.clone())
            .child(dialog_picker.clone())
    });
    picker
}

impl DatabasePicker {
    fn new(
        config: &ConnectionConfig,
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search databases"));
        cx.subscribe_in(
            &search,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => cx.notify(),
                InputEvent::PressEnter { secondary: false, .. } => this.apply(window, cx),
                _ => {}
            },
        )
        .detach();
        let server = ConnectionConfig {
            database: String::new(),
            ..config.clone()
        }
        .summary();
        Self {
            id: config.id.clone(),
            server,
            shown: config.databases.clone(),
            discovery: Discovery::Loading,
            selected: preselected(config),
            search,
            workspace,
            focused_once: false,
        }
    }

    /// The databases the server listed, or why it could not list them.
    pub fn finish(&mut self, result: Result<Vec<String>, String>, cx: &mut Context<Self>) {
        self.discovery = match result {
            Ok(names) => Discovery::Loaded(choices(&names, &self.shown)),
            Err(err) => Discovery::Failed(err),
        };
        cx.notify();
    }

    fn visible(&self, cx: &App) -> Vec<Choice> {
        let Discovery::Loaded(choices) = &self.discovery else {
            return Vec::new();
        };
        let query = self.search.read(cx).value().to_string();
        choices
            .iter()
            .filter(|c| matches(&c.name, &query))
            .cloned()
            .collect()
    }

    fn set_visible(&mut self, checked: bool, cx: &mut Context<Self>) {
        for choice in self.visible(cx) {
            if checked {
                self.selected.insert(choice.name);
            } else {
                self.selected.remove(&choice.name);
            }
        }
        cx.notify();
    }

    fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.discovery, Discovery::Loaded(_)) {
            return;
        }
        let (id, databases) = (self.id.clone(), self.selected.iter().cloned().collect());
        self.workspace
            .update(cx, |ws, cx| ws.show_databases(&id, databases, window, cx))
            .ok();
        window.close_dialog(cx);
    }

    fn render_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let message = |text: String, color: Hsla| div().py_2().text_sm().text_color(color).child(text);
        match &self.discovery {
            Discovery::Loading => {
                message("Discovering databases…".into(), theme.muted_foreground).into_any_element()
            }
            Discovery::Failed(err) => message(err.clone(), theme.danger).into_any_element(),
            Discovery::Loaded(choices) if choices.is_empty() => message(
                format!("{} can connect to no databases.", self.server),
                theme.muted_foreground,
            )
            .into_any_element(),
            Discovery::Loaded(_) => {
                let visible = self.visible(cx);
                if visible.is_empty() {
                    return message("No matching databases".into(), theme.muted_foreground)
                        .into_any_element();
                }
                let rows = visible.into_iter().map(|choice| {
                    let label = if choice.available {
                        choice.name.clone()
                    } else {
                        format!("{} (not found on the server)", choice.name)
                    };
                    let name = choice.name.clone();
                    Checkbox::new(SharedString::from(format!("db-pick-{}", choice.name)))
                        .label(label)
                        .checked(self.selected.contains(&choice.name))
                        .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                            if *checked {
                                this.selected.insert(name.clone());
                            } else {
                                this.selected.remove(&name);
                            }
                            cx.notify();
                        }))
                });
                div()
                    .id("db-pick-list")
                    .max_h(px(320.))
                    .overflow_y_scroll()
                    .child(v_flex().gap_1p5().py_1().children(rows))
                    .into_any_element()
            }
        }
    }
}

impl Render for DatabasePicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.focused_once {
            self.focused_once = true;
            self.search.update(cx, |input, cx| input.focus(window, cx));
        }
        let loaded = matches!(self.discovery, Discovery::Loaded(_));
        let muted = cx.theme().muted_foreground;
        v_flex()
            .gap_3()
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(format!("Pick the databases to show for {}.", self.server)),
            )
            .child(Input::new(&self.search))
            .child(self.render_body(cx))
            .child(
                h_flex()
                    .gap_2()
                    .pt_1()
                    .child(
                        crate::style::dialog_button("db-pick-all")
                            .ghost()
                            .label("Select All")
                            .disabled(!loaded)
                            .on_click(cx.listener(|this, _, _, cx| this.set_visible(true, cx))),
                    )
                    .child(
                        crate::style::dialog_button("db-pick-none")
                            .ghost()
                            .label("Clear")
                            .disabled(!loaded)
                            .on_click(cx.listener(|this, _, _, cx| this.set_visible(false, cx))),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(format!("{} selected", self.selected.len())),
                    )
                    .child(div().flex_1())
                    .child(
                        crate::style::dialog_button("db-pick-cancel")
                            .ghost()
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        crate::style::dialog_button("db-pick-ok")
                            .primary()
                            .label("OK")
                            .disabled(!loaded)
                            .on_click(cx.listener(|this, _, window, cx| this.apply(window, cx))),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use db::ConnectionConfig;

    use super::{choices, matches, preselected};

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn keeps_shown_databases_the_server_no_longer_lists() {
        let list = choices(&names(&["b", "a"]), &names(&["a", "gone"]));
        let got: Vec<(&str, bool)> = list.iter().map(|c| (c.name.as_str(), c.available)).collect();
        assert_eq!(got, [("a", true), ("b", true), ("gone", false)]);
    }

    #[test]
    fn preselects_shown_databases_else_the_own_one() {
        let mut config = ConnectionConfig::default();
        assert!(preselected(&config).is_empty());
        config.database = "app".into();
        assert_eq!(preselected(&config), BTreeSet::from(["app".to_string()]));
        config.databases = names(&["x", "y"]);
        assert_eq!(
            preselected(&config),
            BTreeSet::from(["x".to_string(), "y".to_string()])
        );
    }

    #[test]
    fn search_ignores_case_and_padding() {
        assert!(matches("App_Production", " prod "));
        assert!(matches("anything", ""));
        assert!(!matches("app_test", "prod"));
    }
}
