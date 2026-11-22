//! "Data Source" dialog: create or edit a connection, test it, save it.

use db::{ConnectionConfig, DbKind};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::*;

use crate::workspace::Workspace;

enum TestState {
    Idle,
    Testing,
    Ok,
    Failed(String),
}

pub struct ConnectionForm {
    id: String,
    kind: DbKind,
    tls: bool,
    is_new: bool,
    name: Entity<InputState>,
    host: Entity<InputState>,
    port: Entity<InputState>,
    user: Entity<InputState>,
    password: Entity<InputState>,
    database: Entity<InputState>,
    file: Entity<InputState>,
    test: TestState,
    workspace: WeakEntity<Workspace>,
    focused_once: bool,
}

/// Opens the dialog for `existing`, or for a new data source when `None`.
pub fn open_connection_dialog(
    existing: Option<ConnectionConfig>,
    workspace: WeakEntity<Workspace>,
    window: &mut Window,
    cx: &mut App,
) {
    let form = cx.new(|cx| ConnectionForm::new(existing, workspace, window, cx));
    window.open_dialog(cx, move |dialog, _, cx| {
        let title = if form.read(cx).is_new {
            "New Data Source"
        } else {
            "Edit Data Source"
        };
        dialog.w(px(520.)).title(title).child(form.clone())
    });
}

impl ConnectionForm {
    fn new(
        existing: Option<ConnectionConfig>,
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let is_new = existing.is_none();
        let config = existing.unwrap_or_else(|| ConnectionConfig {
            id: super::new_connection_id(),
            ..Default::default()
        });
        let mut input = |value: String, placeholder: &str| {
            let placeholder = placeholder.to_string();
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .default_value(value)
            })
        };
        let name = input(config.name.clone(), "My database");
        let host = input(config.host.clone(), "localhost");
        let port = input(config.port.to_string(), "port");
        let user = input(config.user.clone(), "user");
        let database = input(config.database.clone(), "database");
        let file = input(config.file.clone(), "/path/to/db.sqlite3");
        let password_placeholder = if is_new { "password" } else { "unchanged" };
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder(password_placeholder)
        });
        Self {
            id: config.id,
            kind: config.kind,
            tls: config.tls,
            is_new,
            name,
            host,
            port,
            user,
            password,
            database,
            file,
            test: TestState::Idle,
            workspace,
            focused_once: false,
        }
    }

    fn value(input: &Entity<InputState>, cx: &App) -> String {
        input.read(cx).value().trim().to_string()
    }

    fn config(&self, cx: &App) -> ConnectionConfig {
        let mut name = Self::value(&self.name, cx);
        if name.is_empty() {
            name = match self.kind {
                DbKind::Sqlite => Self::value(&self.file, cx)
                    .rsplit('/')
                    .next()
                    .unwrap_or("SQLite")
                    .to_string(),
                kind => format!(
                    "{}@{}",
                    kind.label().split(' ').next().unwrap_or("db"),
                    Self::value(&self.host, cx)
                ),
            };
        }
        ConnectionConfig {
            id: self.id.clone(),
            name,
            kind: self.kind,
            host: Self::value(&self.host, cx),
            port: Self::value(&self.port, cx)
                .parse()
                .unwrap_or(self.kind.default_port()),
            user: Self::value(&self.user, cx),
            database: Self::value(&self.database, cx),
            file: Self::value(&self.file, cx),
            tls: self.tls,
        }
    }

    /// Typed password, or the saved one when the field is left empty.
    fn password(&self, cx: &App) -> Option<String> {
        let typed = self.password.read(cx).value().to_string();
        if typed.is_empty() {
            db::load_password(&self.id)
        } else {
            Some(typed)
        }
    }

    fn set_kind(&mut self, kind: DbKind, window: &mut Window, cx: &mut Context<Self>) {
        let old_default = self.kind.default_port().to_string();
        self.kind = kind;
        if Self::value(&self.port, cx) == old_default || Self::value(&self.port, cx).is_empty() {
            let port = kind.default_port().to_string();
            self.port
                .update(cx, |input, cx| input.set_value(port, window, cx));
        }
        self.test = TestState::Idle;
        cx.notify();
    }

    fn test_connection(&mut self, cx: &mut Context<Self>) {
        let (config, password) = (self.config(cx), self.password(cx));
        self.test = TestState::Testing;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(
                    async move { db::Database::connect(&config, password.as_deref()).map(|_| ()) },
                )
                .await;
            this.update(cx, |this, cx| {
                this.test = match result {
                    Ok(()) => TestState::Ok,
                    Err(err) => TestState::Failed(format!("{err:#}")),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let config = self.config(cx);
        let typed_password = self.password.read(cx).value().to_string();
        if !typed_password.is_empty()
            && let Err(err) = db::store_password(&config.id, &typed_password)
        {
            self.test = TestState::Failed(format!("{err:#}"));
            cx.notify();
            return;
        }
        self.workspace
            .update(cx, |ws, cx| ws.save_data_source(config, window, cx))
            .ok();
        window.close_dialog(cx);
    }

    fn choose_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose SQLite database".into()),
        });
        let file = self.file.clone();
        cx.spawn_in(window, async move |_, cx| {
            if let Ok(Ok(Some(paths))) = paths.await
                && let Some(path) = paths.first()
            {
                let path = path.display().to_string();
                file.update_in(cx, |input, window, cx| input.set_value(path, window, cx))
                    .ok();
            }
        })
        .detach();
    }
}

fn field(label: &'static str, input: impl IntoElement) -> Div {
    h_flex()
        .gap_3()
        .child(div().w(px(90.)).flex_none().text_sm().child(label))
        .child(div().flex_1().child(input))
}

impl Render for ConnectionForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.focused_once {
            self.focused_once = true;
            self.name.update(cx, |input, cx| input.focus(window, cx));
        }
        let theme = cx.theme();
        let kinds = h_flex().gap_1().children(DbKind::ALL.map(|kind| {
            let button = Button::new(SharedString::from(format!("kind-{kind:?}")))
                .small()
                .label(kind.label());
            let button = if kind == self.kind {
                button.primary()
            } else {
                button.ghost()
            };
            button.on_click(cx.listener(move |this, _, window, cx| this.set_kind(kind, window, cx)))
        }));

        let connection_fields = if self.kind == DbKind::Sqlite {
            v_flex().gap_2().child(field(
                "File",
                h_flex()
                    .gap_2()
                    .child(div().flex_1().child(Input::new(&self.file)))
                    .child(
                        Button::new("choose-file")
                            .small()
                            .label("Choose…")
                            .on_click(cx.listener(|this, _, window, cx| this.choose_file(window, cx))),
                    ),
            ))
        } else {
            v_flex()
                .gap_2()
                .child(field(
                    "Host",
                    h_flex()
                        .gap_2()
                        .child(div().flex_1().child(Input::new(&self.host)))
                        .child(div().w(px(90.)).child(Input::new(&self.port))),
                ))
                .child(field("User", Input::new(&self.user)))
                .child(field("Password", Input::new(&self.password)))
                .child(field(
                    if self.kind == DbKind::Redis {
                        "DB index"
                    } else {
                        "Database"
                    },
                    Input::new(&self.database),
                ))
                .child(field(
                    "",
                    Checkbox::new("tls")
                        .label("Require TLS")
                        .checked(self.tls)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.tls = *checked;
                            cx.notify();
                        })),
                ))
        };

        let test_status = match &self.test {
            TestState::Idle => None,
            TestState::Testing => Some(div().text_color(theme.muted_foreground).child("Connecting…")),
            TestState::Ok => Some(div().text_color(theme.green).child("✓ Connection succeeded")),
            TestState::Failed(err) => Some(div().text_color(theme.danger).child(err.clone())),
        };

        v_flex()
            .gap_3()
            .child(field("Name", Input::new(&self.name)))
            .child(field("Type", kinds))
            .child(connection_fields)
            .children(test_status.map(|s| s.text_sm()))
            .child(
                h_flex()
                    .gap_2()
                    .pt_2()
                    .child(
                        Button::new("test-connection")
                            .small()
                            .label("Test Connection")
                            .disabled(matches!(self.test, TestState::Testing))
                            .on_click(cx.listener(|this, _, _, cx| this.test_connection(cx))),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("cancel")
                            .ghost()
                            .small()
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("save")
                            .primary()
                            .small()
                            .label("Save")
                            .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                    ),
            )
    }
}
