//! "Remote Site" dialog: create or edit a connection, test it, save it. Passwords and
//! passphrases go to the Keychain; everything else to `remote-sites.toml`.

use std::path::PathBuf;

use gpui_kit::component::button::ButtonVariants;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use remote::secrets::{self, Secret};
use remote::{Auth, ConnectOptions, Credentials, ErrorKind, Protocol, Site};

use super::Workspace;
use crate::style;

/// Label colors offered for a site (theme names; drawn with `site_color`).
pub(super) const COLORS: [&str; 6] = ["red", "yellow", "green", "cyan", "blue", "magenta"];

pub(super) fn site_color(name: Option<&str>, cx: &App) -> Option<Hsla> {
    let theme = cx.theme();
    Some(match name? {
        "red" => theme.red,
        "yellow" => theme.yellow,
        "green" => theme.green,
        "cyan" => theme.cyan,
        "blue" => theme.blue,
        "magenta" => theme.magenta,
        _ => return None,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AuthChoice {
    Password,
    Key,
    Agent,
}

enum TestState {
    Idle,
    Testing,
    Ok,
    Failed(String),
}

pub(super) struct SiteForm {
    site: Site,
    is_new: bool,
    auth: AuthChoice,
    name: Entity<InputState>,
    host: Entity<InputState>,
    port: Entity<InputState>,
    user: Entity<InputState>,
    password: Entity<InputState>,
    key_path: Entity<InputState>,
    passphrase: Entity<InputState>,
    remote_path: Entity<InputState>,
    local_path: Entity<InputState>,
    test: TestState,
    /// "Delete Site" was clicked once; the second click deletes.
    confirm_delete: bool,
    workspace: WeakEntity<Workspace>,
    focused_once: bool,
}

impl Workspace {
    /// New Remote Site…: an empty form (the project folder as local folder).
    pub(in crate::workspace) fn new_remote_site(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ensure_remote(window, cx);
        self.right_panel = Some(super::super::right_sidebar::RightPanel::Remote);
        let mut site = Site::new("", Protocol::Sftp, "", "");
        site.local_path = (!self.light).then(|| self.root.clone());
        self.open_site_form(site, true, window, cx);
    }

    pub(super) fn edit_remote_site(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(site) = self.remote.as_ref().and_then(|s| s.site(id)).cloned() {
            self.open_site_form(site, false, window, cx);
        }
    }

    pub(super) fn open_site_form(
        &mut self,
        site: Site,
        is_new: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let workspace = cx.entity().downgrade();
        let form = cx.new(|cx| SiteForm::new(site, is_new, workspace, window, cx));
        window.open_dialog(cx, move |dialog, _, cx| {
            let title = if form.read(cx).is_new {
                "New Remote Site"
            } else {
                "Edit Remote Site"
            };
            dialog.w(px(560.)).title(title).child(form.clone())
        });
    }
}

impl SiteForm {
    fn new(
        site: Site,
        is_new: bool,
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut input = |value: String, placeholder: &str, masked: bool| {
            let placeholder = placeholder.to_string();
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .masked(masked)
                    .default_value(value)
            })
        };
        let key_path = match &site.auth {
            Auth::Key { path } => path.display().to_string(),
            _ => String::new(),
        };
        let secret_hint = if is_new { "" } else { "unchanged" };
        let name = input(site.name.clone(), "Production server", false);
        let host = input(site.host.clone(), "example.com or 1.2.3.4", false);
        let port = input(site.port.to_string(), "port", false);
        let user = input(site.user.clone(), "user", false);
        let password = input(String::new(), if is_new { "password" } else { secret_hint }, true);
        let key_path = input(key_path, "~/.ssh/id_ed25519", false);
        let passphrase = input(
            String::new(),
            if is_new {
                "passphrase (if any)"
            } else {
                secret_hint
            },
            true,
        );
        let remote_path = input(site.remote_path.clone(), "/var/www (empty: home folder)", false);
        let local_path = input(
            site.local_path
                .as_ref()
                .map_or_default(|p| p.display().to_string()),
            "local folder (optional)",
            false,
        );
        for input in [
            &name,
            &host,
            &port,
            &user,
            &password,
            &key_path,
            &passphrase,
            &remote_path,
            &local_path,
        ] {
            cx.subscribe_in(input, window, |this, _, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { secondary: false, .. } = event {
                    this.save(window, cx);
                }
            })
            .detach();
        }
        let auth = match site.auth {
            Auth::Password => AuthChoice::Password,
            Auth::Key { .. } => AuthChoice::Key,
            Auth::Agent => AuthChoice::Agent,
        };
        Self {
            site,
            is_new,
            auth,
            name,
            host,
            port,
            user,
            password,
            key_path,
            passphrase,
            remote_path,
            local_path,
            test: TestState::Idle,
            confirm_delete: false,
            workspace,
            focused_once: false,
        }
    }

    fn value(input: &Entity<InputState>, cx: &App) -> String {
        input.read(cx).value().trim().to_string()
    }

    /// The site as typed (secrets excluded).
    fn typed_site(&self, cx: &App) -> Site {
        let mut site = self.site.clone();
        site.name = Self::value(&self.name, cx);
        site.host = Self::value(&self.host, cx);
        site.port = Self::value(&self.port, cx)
            .parse()
            .unwrap_or(site.protocol.default_port());
        site.user = Self::value(&self.user, cx);
        let key = Self::value(&self.key_path, cx);
        site.auth = match (site.protocol, self.auth) {
            (Protocol::Sftp, AuthChoice::Agent) => Auth::Agent,
            (Protocol::Sftp, AuthChoice::Key) => Auth::Key {
                path: expand_home(&key),
            },
            _ => Auth::Password,
        };
        site.remote_path = Self::value(&self.remote_path, cx)
            .trim_end_matches('/')
            .to_string();
        if site.remote_path.is_empty() && Self::value(&self.remote_path, cx) == "/" {
            site.remote_path = "/".into();
        }
        let local = Self::value(&self.local_path, cx);
        site.local_path = (!local.is_empty()).then(|| expand_home(&local));
        if site.name.is_empty() {
            site.name.clone_from(&site.host);
        }
        site
    }

    /// Typed secrets, falling back to the Keychain for fields left empty.
    fn credentials(&self, cx: &App) -> Credentials {
        let typed =
            |input: &Entity<InputState>| Some(input.read(cx).value().to_string()).filter(|v| !v.is_empty());
        let stored = Credentials::from_keychain(&self.site);
        Credentials {
            password: typed(&self.password).or(stored.password),
            passphrase: typed(&self.passphrase).or(stored.passphrase),
        }
    }

    fn set_protocol(&mut self, protocol: Protocol, window: &mut Window, cx: &mut Context<Self>) {
        let old_default = self.site.protocol.default_port().to_string();
        self.site.protocol = protocol;
        let port = Self::value(&self.port, cx);
        if port == old_default || port.is_empty() {
            let port = protocol.default_port().to_string();
            self.port
                .update(cx, |input, cx| input.set_value(port, window, cx));
        }
        if protocol != Protocol::Sftp {
            self.auth = AuthChoice::Password;
        }
        self.test = TestState::Idle;
        cx.notify();
    }

    fn test_connection(&mut self, cx: &mut Context<Self>) {
        let site = self.typed_site(cx);
        if let Some(problem) = site.validate() {
            self.test = TestState::Failed(problem.into());
            return cx.notify();
        }
        let credentials = self.credentials(cx);
        self.test = TestState::Testing;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let fs = remote::connect(&site, &credentials, &ConnectOptions::default())?;
                    fs.home()
                })
                .await;
            this.update(cx, |this, cx| {
                this.test = match result {
                    Ok(_) => TestState::Ok,
                    Err(err) => TestState::Failed(match err.kind {
                        // A test never trusts keys; connecting from the panel asks.
                        ErrorKind::HostKeyUnknown(key) => format!(
                            "Reached the server. Its host key ({}) is not trusted yet: Connect from the panel to confirm it.",
                            key.fingerprint
                        ),
                        _ => err.user_message(),
                    }),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let site = self.typed_site(cx);
        if let Some(problem) = site.validate() {
            self.test = TestState::Failed(problem.into());
            return cx.notify();
        }
        for (input, secret) in [
            (&self.password, Secret::Password),
            (&self.passphrase, Secret::Passphrase),
        ] {
            let typed = input.read(cx).value().to_string();
            if typed.is_empty() {
                continue;
            }
            if let Err(err) = secrets::store(&site.id, secret, &typed) {
                self.test = TestState::Failed(format!("{err:#}"));
                return cx.notify();
            }
        }
        self.workspace
            .update(cx, |ws, cx| ws.upsert_remote_site(site, window, cx))
            .ok();
        window.close_dialog(cx);
    }

    fn delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.confirm_delete {
            self.confirm_delete = true;
            return cx.notify();
        }
        let id = self.site.id.clone();
        self.workspace
            .update(cx, |ws, cx| ws.delete_remote_site(&id, window, cx))
            .ok();
        window.close_dialog(cx);
    }

    fn choose_path(
        &mut self,
        target: Entity<InputState>,
        directories: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: !directories,
            directories,
            multiple: false,
            prompt: Some("Choose".into()),
        });
        cx.spawn_in(window, async move |_, cx| {
            if let Ok(Ok(Some(paths))) = paths.await
                && let Some(path) = paths.first()
            {
                let path = path.display().to_string();
                target
                    .update_in(cx, |input, window, cx| input.set_value(path, window, cx))
                    .ok();
            }
        })
        .detach();
    }
}

fn expand_home(text: &str) -> PathBuf {
    match text.strip_prefix("~/") {
        Some(rest) => dirs::home_dir().unwrap_or_default().join(rest),
        None => PathBuf::from(text),
    }
}

fn field(label: &'static str, input: impl IntoElement) -> Div {
    h_flex()
        .gap_3()
        .child(div().w(px(100.)).flex_none().text_sm().child(label))
        .child(div().flex_1().child(input))
}

impl Render for SiteForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.focused_once {
            self.focused_once = true;
            self.name.update(cx, |input, cx| input.focus(window, cx));
        }
        let theme = cx.theme();
        let protocol = self.site.protocol;
        let protocols = h_flex().gap_1().children(Protocol::ALL.map(|p| {
            let button =
                style::dialog_button(SharedString::from(format!("protocol-{}", p.label()))).label(p.label());
            let button = if p == protocol {
                button.primary()
            } else {
                button.ghost()
            };
            button.on_click(cx.listener(move |this, _, window, cx| this.set_protocol(p, window, cx)))
        }));
        let auth_choice = self.auth;
        let auths = h_flex().gap_1().children(
            [
                (AuthChoice::Agent, "ssh-agent"),
                (AuthChoice::Key, "Private key"),
                (AuthChoice::Password, "Password"),
            ]
            .map(|(choice, label)| {
                let button = style::dialog_button(SharedString::from(format!("auth-{label}"))).label(label);
                let button = if choice == auth_choice {
                    button.primary()
                } else {
                    button.ghost()
                };
                button.on_click(cx.listener(move |this, _, _, cx| {
                    this.auth = choice;
                    this.test = TestState::Idle;
                    cx.notify();
                }))
            }),
        );
        let key_path = self.key_path.clone();
        let local_path = self.local_path.clone();
        let colors = h_flex().gap_1p5().children(COLORS.map(|name| {
            let color = site_color(Some(name), cx).unwrap_or(theme.muted_foreground);
            let selected = self.site.color.as_deref() == Some(name);
            div()
                .id(SharedString::from(format!("color-{name}")))
                .size(px(18.))
                .rounded_full()
                .bg(color)
                .cursor_pointer()
                .border_2()
                .border_color(if selected {
                    theme.foreground
                } else {
                    theme.transparent
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.site.color = if this.site.color.as_deref() == Some(name) {
                        None
                    } else {
                        Some(name.to_string())
                    };
                    cx.notify();
                }))
        }));
        let test_status = match &self.test {
            TestState::Idle => None,
            TestState::Testing => Some(div().text_color(theme.muted_foreground).child("Connecting…")),
            TestState::Ok => Some(div().text_color(theme.green).child("✓ Connection succeeded")),
            TestState::Failed(err) => Some(div().text_color(theme.danger).child(err.clone())),
        };
        let sftp = protocol == Protocol::Sftp;
        let uses_password = !sftp || auth_choice == AuthChoice::Password;
        v_flex()
            .gap_3()
            .child(field("Name", Input::new(&self.name)))
            .child(field("Protocol", protocols))
            .child(field(
                "Host",
                h_flex()
                    .gap_2()
                    .child(div().flex_1().child(Input::new(&self.host)))
                    .child(div().w(px(80.)).child(Input::new(&self.port))),
            ))
            .child(field("User", Input::new(&self.user)))
            .when(sftp, |el| el.child(field("Login", auths)))
            .when(uses_password, |el| {
                el.child(field("Password", Input::new(&self.password)))
            })
            .when(sftp && auth_choice == AuthChoice::Key, |el| {
                el.child(field(
                    "Key file",
                    h_flex()
                        .gap_2()
                        .child(div().flex_1().child(Input::new(&self.key_path)))
                        .child(
                            style::dialog_button("choose-key")
                                .label("Choose…")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.choose_path(key_path.clone(), false, window, cx);
                                })),
                        ),
                ))
                .child(field("Passphrase", Input::new(&self.passphrase)))
            })
            .child(field("Remote path", Input::new(&self.remote_path)))
            .child(field(
                "Local folder",
                h_flex()
                    .gap_2()
                    .child(div().flex_1().child(Input::new(&self.local_path)))
                    .child(
                        style::dialog_button("choose-local")
                            .label("Choose…")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.choose_path(local_path.clone(), true, window, cx);
                            })),
                    ),
            ))
            .child(field("Color", colors))
            .child(field(
                "",
                Checkbox::new("ai-read")
                    .label("Let AI agents list and read files here (MCP, read-only)")
                    .checked(self.site.ai_read_access)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.site.ai_read_access = *checked;
                        cx.notify();
                    })),
            ))
            .child(
                div().text_xs().text_color(theme.muted_foreground).child(
                    "Passwords and passphrases are stored in the macOS Keychain, never in config files.",
                ),
            )
            .children(test_status.map(Styled::text_sm))
            .child(
                h_flex()
                    .gap_2()
                    .pt_2()
                    .child(
                        style::dialog_button("test-connection")
                            .label("Test Connection")
                            .disabled(matches!(self.test, TestState::Testing))
                            .on_click(cx.listener(|this, _, _, cx| this.test_connection(cx))),
                    )
                    .when(!self.is_new, |row| {
                        row.child(
                            style::dialog_button("delete-site")
                                .when(self.confirm_delete, ButtonVariants::danger)
                                .when(!self.confirm_delete, ButtonVariants::ghost)
                                .label(if self.confirm_delete {
                                    "Delete Site and Its Secrets?"
                                } else {
                                    "Delete Site"
                                })
                                .on_click(cx.listener(|this, _, window, cx| this.delete(window, cx))),
                        )
                    })
                    .child(div().flex_1())
                    .child(
                        style::dialog_button("cancel")
                            .ghost()
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        style::dialog_button("save")
                            .primary()
                            .label("Save")
                            .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                    ),
            )
    }
}
