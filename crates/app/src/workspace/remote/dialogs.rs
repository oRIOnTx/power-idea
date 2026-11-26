//! Remote dialogs: confirming a host key, resolving an upload conflict, chmod, delete.

use gpui_kit::component::button::{ButtonVariant, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::*;
use remote::HostKey;
use remote::conflict::Conflict;
use remote::listing::{format_mode, parse_octal_mode};

use super::Workspace;
use crate::style;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ConflictChoice {
    Overwrite,
    Reload,
    ShowDiff,
}

impl Workspace {
    /// Shows the server's fingerprint; nothing is trusted without an explicit click.
    /// `changed_line` is set when known_hosts holds a different key for the host.
    pub(super) fn confirm_host_key(
        &mut self,
        site: &str,
        key: HostKey,
        changed_line: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let workspace = cx.entity().downgrade();
        let site = site.to_string();
        window.open_alert_dialog(cx, move |dialog, _, cx| {
            let (workspace, site, key) = (workspace.clone(), site.clone(), key.clone());
            let theme = cx.theme();
            let host = if key.port == 22 {
                key.host.clone()
            } else {
                format!("{}:{}", key.host, key.port)
            };
            let fingerprint = div()
                .p_2()
                .rounded_md()
                .bg(theme.muted)
                .font_family("monospace")
                .text_sm()
                .child(format!("{} {}", key.algorithm, key.fingerprint));
            let dialog = match changed_line {
                None => dialog
                    .confirm()
                    .width(px(520.))
                    .icon(style::lucide("key-round").text_color(theme.primary))
                    .title(format!("Trust the host key of {host}?"))
                    .child(
                        v_flex()
                            .gap_2()
                            .child("This is the first connection to this server. Compare the fingerprint with the one your hosting provider shows (or `ssh-keygen -lf /etc/ssh/ssh_host_*_key.pub` on the server).")
                            .child(fingerprint)
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child("Trusting adds the key to ~/.ssh/known_hosts, like answering “yes” to ssh."),
                            ),
                    )
                    .footer(style::confirm_buttons("Trust and Connect", ButtonVariant::Primary)),
                Some(line) => style::danger_confirm(dialog, "shield-alert", "Connect Anyway", cx)
                    .width(px(540.))
                    .title(format!("The host key of {host} changed"))
                    .child(
                        v_flex()
                            .gap_2()
                            .child(format!(
                                "~/.ssh/known_hosts (line {line}) has a different key for this server. \
                                 The server may have been reinstalled — or someone may be intercepting the connection."
                            ))
                            .child(fingerprint)
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(format!(
                                        "“Connect Anyway” trusts this key for this session only. If the change is \
                                         expected, run `ssh-keygen -R {}` and connect again.",
                                        key.host
                                    )),
                            ),
                    ),
            };
            let changed = changed_line.is_some();
            dialog.on_ok(move |_, window, cx| {
                workspace
                    .update(cx, |ws, cx| ws.host_key_accepted(&site, key.clone(), changed, window, cx))
                    .ok();
                true
            })
        });
    }

    /// The server copy changed (or vanished) since the tab opened it.
    pub(super) fn ask_remote_conflict(
        &mut self,
        key: &str,
        name: &str,
        site_name: &str,
        found: Conflict,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let workspace = cx.entity().downgrade();
        let key = key.to_string();
        let title = match found {
            Conflict::Deleted => format!("“{name}” was deleted on {site_name}"),
            _ => format!("“{name}” changed on {site_name}"),
        };
        let body = match found {
            Conflict::Deleted => "Uploading creates it again with your text.",
            _ => {
                "Someone changed the file on the server since you opened it. Your changes are kept until you choose."
            }
        };
        window.open_dialog(cx, move |dialog, _, cx| {
            let choose = |choice: ConflictChoice| {
                let (workspace, key) = (workspace.clone(), key.clone());
                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    window.close_dialog(cx);
                    workspace
                        .update(cx, |ws, cx| ws.resolve_remote_conflict(&key, choice, window, cx))
                        .ok();
                }
            };
            dialog
                .w(px(500.))
                .title(title.clone())
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(body),
                )
                .footer(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(
                            style::dialog_button("conflict-diff")
                                .label("Show Diff")
                                .on_click(choose(ConflictChoice::ShowDiff)),
                        )
                        .child(div().flex_1())
                        .child(
                            style::dialog_button("conflict-cancel")
                                .ghost()
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            style::dialog_button("conflict-reload")
                                .label("Reload from Server")
                                .on_click(choose(ConflictChoice::Reload)),
                        )
                        .child(
                            style::dialog_button("conflict-overwrite")
                                .danger()
                                .label("Overwrite")
                                .on_click(choose(ConflictChoice::Overwrite)),
                        ),
                )
        });
    }

    /// Deleting on a server is permanent (there is no Trash): always ask.
    pub(super) fn confirm_remote_delete(
        &mut self,
        site: &str,
        path: &str,
        is_dir: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let workspace = cx.entity().downgrade();
        let (site, path) = (site.to_string(), path.to_string());
        let name = remote::path::file_name(&path).to_string();
        window.open_alert_dialog(cx, move |dialog, _, cx| {
            let (workspace, site, path) = (workspace.clone(), site.clone(), path.clone());
            style::danger_confirm(dialog, "trash", "Delete", cx)
                .title(format!("Delete “{name}” on the server?"))
                .child(if is_dir {
                    format!("{path} and everything in it are deleted permanently.")
                } else {
                    format!("{path} is deleted permanently.")
                })
                .on_ok(move |_, window, cx| {
                    workspace
                        .update(cx, |ws, cx| ws.delete_remote(&site, &path, window, cx))
                        .ok();
                    true
                })
        });
    }

    pub(super) fn open_chmod_dialog(
        &mut self,
        site: &str,
        path: &str,
        mode: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let workspace = cx.entity().downgrade();
        let form = cx.new(|cx| ChmodForm::new(site, path, mode, workspace, window, cx));
        let name = remote::path::file_name(path).to_string();
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .w(px(420.))
                .title(format!("Permissions of “{name}”"))
                .child(form.clone())
        });
    }
}

/// Nine checkboxes and an octal field, kept in sync.
pub(super) struct ChmodForm {
    site: String,
    path: String,
    mode: u32,
    octal: Entity<InputState>,
    error: Option<String>,
    workspace: WeakEntity<Workspace>,
}

impl ChmodForm {
    fn new(
        site: &str,
        path: &str,
        mode: u32,
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let octal = cx.new(|cx| InputState::new(window, cx).default_value(format!("{:03o}", mode & 0o777)));
        cx.subscribe_in(
            &octal,
            window,
            |this, input, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    let text = input.read(cx).value().to_string();
                    match parse_octal_mode(&text) {
                        Some(mode) => {
                            this.mode = mode;
                            this.error = None;
                        }
                        None => this.error = Some("Enter an octal mode such as 644 or 0755".into()),
                    }
                    cx.notify();
                }
                InputEvent::PressEnter { secondary: false, .. } => this.apply(window, cx),
                _ => {}
            },
        )
        .detach();
        Self {
            site: site.to_string(),
            path: path.to_string(),
            mode: mode & 0o7777,
            octal,
            error: None,
            workspace,
        }
    }

    fn toggle(&mut self, bit: u32, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        if on {
            self.mode |= bit;
        } else {
            self.mode &= !bit;
        }
        let text = format!("{:03o}", self.mode);
        self.octal
            .update(cx, |input, cx| input.set_value(text, window, cx));
        self.error = None;
        cx.notify();
    }

    fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.error.is_some() {
            return;
        }
        let (site, path, mode) = (self.site.clone(), self.path.clone(), self.mode);
        self.workspace
            .update(cx, |ws, cx| ws.chmod_remote(&site, &path, mode, window, cx))
            .ok();
        window.close_dialog(cx);
    }
}

impl Render for ChmodForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let header = h_flex()
            .gap_2()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(div().w(px(70.)))
            .children(["Read", "Write", "Execute"].map(|t| div().w(px(70.)).child(t)));
        let rows = [("Owner", 6u32), ("Group", 3), ("Others", 0)].map(|(who, shift)| {
            h_flex()
                .gap_2()
                .child(div().w(px(70.)).text_sm().child(who))
                .children([(4u32, "r"), (2, "w"), (1, "x")].map(|(bit, letter)| {
                    let bit = bit << shift;
                    div().w(px(70.)).child(
                        Checkbox::new(SharedString::from(format!("chmod-{who}-{letter}")))
                            .checked(self.mode & bit != 0)
                            .on_click(cx.listener(move |this, checked: &bool, window, cx| {
                                this.toggle(bit, *checked, window, cx);
                            })),
                    )
                }))
        });
        v_flex()
            .gap_2()
            .child(header)
            .children(rows)
            .child(
                h_flex()
                    .gap_2()
                    .pt_2()
                    .child(div().w(px(70.)).text_sm().child("Octal"))
                    .child(div().w(px(90.)).child(Input::new(&self.octal)))
                    .child(
                        div()
                            .text_sm()
                            .font_family("monospace")
                            .text_color(theme.muted_foreground)
                            .child(format_mode(self.mode)),
                    ),
            )
            .children(
                self.error
                    .clone()
                    .map(|e| div().text_sm().text_color(theme.danger).child(e)),
            )
            .child(
                h_flex()
                    .gap_2()
                    .pt_2()
                    .justify_end()
                    .child(
                        style::dialog_button("chmod-cancel")
                            .ghost()
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        style::dialog_button("chmod-apply")
                            .primary()
                            .label("Apply")
                            .on_click(cx.listener(|this, _, window, cx| this.apply(window, cx))),
                    ),
            )
    }
}
