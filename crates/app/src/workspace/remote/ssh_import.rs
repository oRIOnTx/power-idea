//! Import from ~/.ssh/config: pick a host, get a prefilled site form.

use gpui_kit::component::{ActiveTheme as _, WindowExt as _, h_flex};
use gpui_kit::*;
use remote::ssh_config::SshHost;
use remote::{Auth, Protocol, Site};

use super::Workspace;
use crate::style;

impl Workspace {
    /// Import from ~/.ssh/config: one click per host fills a new site form.
    pub(super) fn import_ssh_hosts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let hosts = remote::ssh_config::load_user_config();
        if hosts.is_empty() {
            return self.notify_error("No hosts found in ~/.ssh/config".into(), window, cx);
        }
        let workspace = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            let theme = cx.theme();
            let rows = hosts.iter().enumerate().map(|(ix, host)| {
                let (workspace, host) = (workspace.clone(), host.clone());
                let detail = format!(
                    "{}{}{}",
                    host.user.as_ref().map_or_default(|u| format!("{u}@")),
                    host.hostname,
                    host.port.map_or_default(|p| format!(":{p}"))
                );
                h_flex()
                    .id(("ssh-host", ix))
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.list_hover))
                    .child(style::lucide("server").size_4())
                    .child(div().text_sm().child(host.alias.clone()))
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(detail),
                    )
                    .on_click(move |_, window, cx| {
                        window.close_dialog(cx);
                        let site = site_from_ssh(&host);
                        workspace
                            .update(cx, |ws, cx| ws.open_site_form(site, true, window, cx))
                            .ok();
                    })
            });
            dialog.w(px(480.)).title("Import from ~/.ssh/config").child(
                div()
                    .id("ssh-hosts")
                    .max_h(px(360.))
                    .overflow_y_scroll()
                    .children(rows),
            )
        });
    }
}

fn site_from_ssh(host: &SshHost) -> Site {
    let mut site = Site::new(
        &host.alias,
        Protocol::Sftp,
        &host.hostname,
        host.user.as_deref().unwrap_or(""),
    );
    if let Some(port) = host.port {
        site.port = port;
    }
    site.auth = match &host.identity_file {
        Some(path) => Auth::Key { path: path.clone() },
        None => Auth::Agent,
    };
    site
}

#[cfg(test)]
mod tests {
    use remote::Auth;
    use remote::ssh_config::SshHost;

    use super::site_from_ssh;

    #[test]
    fn imports_ssh_hosts() {
        let host = SshHost {
            alias: "prod".into(),
            hostname: "1.2.3.4".into(),
            user: Some("deploy".into()),
            port: Some(2222),
            identity_file: Some("/k".into()),
        };
        let site = site_from_ssh(&host);
        assert_eq!(
            (site.name.as_str(), site.host.as_str(), site.port),
            ("prod", "1.2.3.4", 2222)
        );
        assert_eq!(site.auth, Auth::Key { path: "/k".into() });
        let agent = site_from_ssh(&SshHost {
            identity_file: None,
            ..host
        });
        assert_eq!(agent.auth, Auth::Agent);
    }
}
