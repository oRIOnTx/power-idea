//! Remote tree context menu actions: open, transfer, create, rename, delete, chmod,
//! copy the path, and ssh terminals (shell in a folder, `tail -F` of a log).

use std::collections::BTreeMap;

use anyhow::bail;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;
use remote::path::{self, shell_quote};
use remote::{Auth, Protocol, Site};

use super::Workspace;
use crate::actions::{FileAction, FileOp, RemoteAction, RemoteOp};

impl Workspace {
    pub(in crate::workspace) fn handle_remote_action(
        &mut self,
        action: &RemoteAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ensure_remote(window, cx);
        let (site, target, is_dir) = (action.site.to_string(), action.path.to_string(), action.is_dir);
        // New items go into the clicked folder, or the clicked file's folder.
        let dir = if is_dir {
            target.clone()
        } else {
            path::parent(&target)
        };
        match action.op {
            RemoteOp::Open if is_dir => self.toggle_remote_dir(&site, &target, window, cx),
            RemoteOp::Open => self.open_remote_file(&site, &target, window, cx),
            RemoteOp::GoInto => self.set_remote_root(&site, &dir, window, cx),
            RemoteOp::Refresh => self.load_remote_dir(&site, &dir, window, cx),
            RemoteOp::DownloadTo => self.download_remote_to(&site, &target, window, cx),
            RemoteOp::UploadHere => self.upload_into_remote(&site, &dir, window, cx),
            RemoteOp::NewFile => self.create_remote(&site, &dir, false, window, cx),
            RemoteOp::NewFolder => self.create_remote(&site, &dir, true, window, cx),
            RemoteOp::Rename => self.rename_remote(&site, &target, window, cx),
            RemoteOp::Delete => self.confirm_remote_delete(&site, &target, is_dir, window, cx),
            RemoteOp::CopyPath => {
                cx.write_to_clipboard(ClipboardItem::new_string(target.clone()));
                window.push_notification(Notification::info(target).title("Copied"), cx);
            }
            RemoteOp::Permissions => {
                let mode = self.remote_entry(&site, &target).and_then(|e| e.permissions);
                self.open_chmod_dialog(&site, &target, mode.unwrap_or(0o644), window, cx);
            }
            RemoteOp::OpenTerminal => {
                let command = format!("cd {} && exec \"$SHELL\" -l", shell_quote(&dir));
                self.ssh_terminal(&site, path::file_name(&dir), &command, window, cx);
            }
            RemoteOp::TailLog => {
                let command = format!("tail -n 200 -F {}", shell_quote(&target));
                let name = format!("tail {}", path::file_name(&target));
                self.ssh_terminal(&site, &name, &command, window, cx);
            }
            RemoteOp::Reveal => {
                self.reveal_remote(&site, &target, window, cx);
            }
            RemoteOp::Reload => {
                if let Some(rel) = self
                    .remote_site_name(&site)
                    .map(|n| crate::editor::remote_rel(&n, &target))
                {
                    self.reload_remote_tab(&rel, window, cx);
                }
            }
        }
    }

    fn remote_site_name(&self, site: &str) -> Option<String> {
        self.remote.as_ref()?.site(site).map(|s| s.name.clone())
    }

    /// Shows `target` in the Remote Files panel: its folder becomes the root.
    fn reveal_remote(&mut self, site: &str, target: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.show_project_view = true;
        self.left_panel = super::LeftPanel::Remote;
        self.select_remote_site(site, window, cx);
        let site_id = site.to_string();
        let (dir, target) = (path::parent(target), target.to_string());
        self.connect_remote_then(
            site,
            window,
            cx,
            Some(Box::new(move |ws, conn, window, cx| {
                if conn.is_ok() {
                    ws.set_remote_root(&site_id, &dir, window, cx);
                    if let Some(state) = ws.remote.as_mut() {
                        state.selected = Some(target);
                    }
                }
            })),
        );
    }

    fn create_remote(
        &mut self,
        site: &str,
        dir: &str,
        folder: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let title = if folder { "New Folder" } else { "New File" };
        let (site, dir) = (site.to_string(), dir.to_string());
        self.prompt_name(title, "", window, cx, move |ws, name, window, cx| {
            let name = match path::validate_name(&name) {
                Ok(name) => name.to_string(),
                Err(problem) => bail!(problem),
            };
            let target = path::join(&dir, &name);
            let (site_id, parent, created) = (site.clone(), dir.clone(), target.clone());
            ws.run_remote(
                &site,
                if folder {
                    "Creating a folder"
                } else {
                    "Creating a file"
                },
                move |conn| {
                    conn.run(|fs| {
                        if fs.exists(&target)? {
                            return Err(remote::RemoteError::other(format!("{target} already exists")));
                        }
                        if folder {
                            fs.mkdir(&target)
                        } else {
                            fs.write(&target, b"")
                        }
                    })
                },
                move |ws, (), window, cx| {
                    ws.load_remote_dir(&site_id, &parent, window, cx);
                    if !folder {
                        ws.open_remote_file(&site_id, &created, window, cx);
                    }
                },
                window,
                cx,
            );
            Ok(())
        });
    }

    fn rename_remote(&mut self, site: &str, target: &str, window: &mut Window, cx: &mut Context<Self>) {
        let current = path::file_name(target).to_string();
        let (site, target) = (site.to_string(), target.to_string());
        self.prompt_name("Rename", &current, window, cx, move |ws, name, window, cx| {
            let name = match path::validate_name(&name) {
                Ok(name) => name.to_string(),
                Err(problem) => bail!(problem),
            };
            let parent = path::parent(&target);
            let renamed = path::join(&parent, &name);
            if renamed == target {
                return Ok(());
            }
            let (from, to, site_id) = (target.clone(), renamed.clone(), site.clone());
            ws.run_remote(
                &site,
                "Renaming",
                move |conn| {
                    conn.run(|fs| {
                        if fs.exists(&to)? {
                            return Err(remote::RemoteError::other(format!("{to} already exists")));
                        }
                        fs.rename(&from, &to)
                    })
                },
                move |ws, (), window, cx| ws.load_remote_dir(&site_id, &parent, window, cx),
                window,
                cx,
            );
            Ok(())
        });
    }

    pub(super) fn delete_remote(
        &mut self,
        site: &str,
        target: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (site_id, parent, gone) = (site.to_string(), path::parent(target), target.to_string());
        let doomed = target.to_string();
        self.run_remote(
            site,
            "Deleting",
            move |conn| conn.run(|fs| fs.delete(&doomed)),
            move |ws, (), window, cx| {
                if let Some(conn) = ws.remote.as_mut().and_then(|s| s.conns.get_mut(&site_id)) {
                    conn.expanded.retain(|p| !path::is_within(&gone, p));
                    conn.dirs.retain(|p, _| !path::is_within(&gone, p));
                }
                ws.load_remote_dir(&site_id, &parent, window, cx);
            },
            window,
            cx,
        );
    }

    pub(super) fn chmod_remote(
        &mut self,
        site: &str,
        target: &str,
        mode: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (site_id, parent, changed) = (site.to_string(), path::parent(target), target.to_string());
        self.run_remote(
            site,
            "Changing permissions",
            move |conn| conn.run(|fs| fs.chmod(&changed, mode)),
            move |ws, (), window, cx| ws.load_remote_dir(&site_id, &parent, window, cx),
            window,
            cx,
        );
    }

    /// Download to…: asks for a local folder (the site's local folder by default).
    fn download_remote_to(&mut self, site: &str, target: &str, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Download Here".into()),
        });
        let (site, target) = (site.to_string(), target.to_string());
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await
                && let Some(dir) = paths.into_iter().next()
            {
                this.update_in(cx, |ws, window, cx| {
                    ws.queue_download(&site, &target, dir, window, cx);
                })
                .ok();
            }
        })
        .detach();
    }

    /// Upload here…: asks for local files or folders.
    fn upload_into_remote(&mut self, site: &str, dir: &str, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: true,
            multiple: true,
            prompt: Some("Upload".into()),
        });
        let (site, dir) = (site.to_string(), dir.to_string());
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await {
                this.update_in(cx, |ws, window, cx| {
                    for local in paths {
                        ws.queue_upload(&site, local, &dir, window, cx);
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    /// Runs `remote_command` over the system `ssh` in a terminal tab (SFTP sites only).
    fn ssh_terminal(
        &mut self,
        site: &str,
        name: &str,
        remote_command: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(site) = self.remote.as_ref().and_then(|s| s.site(site)).cloned() else {
            return;
        };
        let Some(command) = ssh_command(&site, remote_command) else {
            return self.notify_error("Terminals need an SFTP (SSH) site".into(), window, cx);
        };
        let root = self.root.clone();
        self.open_run_tab(
            format!("{}: {name}", site.name),
            &command,
            root,
            &BTreeMap::new(),
            window,
            cx,
        );
    }

    /// The context menu of a remote tree row.
    pub(super) fn remote_row_menu(&self, menu: PopupMenu, site: &Site, entry: &remote::Entry) -> PopupMenu {
        let action = |op| RemoteAction::boxed(op, &site.id, &entry.path, entry.is_dir());
        let ssh = site.protocol == Protocol::Sftp;
        let menu = menu.action_context(self.focus_handle.clone());
        let menu = if entry.is_dir() {
            menu.menu("Open as Root", action(RemoteOp::GoInto))
        } else {
            menu.menu("Open", action(RemoteOp::Open))
        };
        menu.menu("Download to…", action(RemoteOp::DownloadTo))
            .menu("Upload Here…", action(RemoteOp::UploadHere))
            .separator()
            .menu("New File…", action(RemoteOp::NewFile))
            .menu("New Folder…", action(RemoteOp::NewFolder))
            .separator()
            .menu("Rename…", action(RemoteOp::Rename))
            .menu("Delete…", action(RemoteOp::Delete))
            .separator()
            .menu("Copy Remote Path", action(RemoteOp::CopyPath))
            .menu("Permissions…", action(RemoteOp::Permissions))
            .menu("Refresh", action(RemoteOp::Refresh))
            .separator()
            .menu_with_disabled("Open Terminal Here", action(RemoteOp::OpenTerminal), !ssh)
            .menu_with_disabled(
                "Tail Log (tail -F)",
                action(RemoteOp::TailLog),
                !ssh || entry.is_dir(),
            )
    }

    /// The context menu of a remote editor tab.
    pub(in crate::workspace) fn remote_tab_menu(
        &self,
        menu: PopupMenu,
        rel: &str,
        site: &str,
        target: &str,
    ) -> PopupMenu {
        let action = |op| RemoteAction::boxed(op, site, target, false);
        let tab = |op| FileAction::boxed(op, rel.to_string());
        menu.action_context(self.focus_handle.clone())
            .menu("Close", tab(FileOp::CloseTab))
            .menu("Close Other Tabs", tab(FileOp::CloseOtherTabs))
            .menu("Close All Tabs", tab(FileOp::CloseAllTabs))
            .separator()
            .menu("Copy Remote Path", action(RemoteOp::CopyPath))
            .menu("Show in Remote Files", action(RemoteOp::Reveal))
            .menu("Reload from Server", action(RemoteOp::Reload))
            .menu("Download to…", action(RemoteOp::DownloadTo))
    }
}

/// `ssh -t [-p port] [-i key] user@host '<command>'`, or `None` for FTP sites.
fn ssh_command(site: &Site, remote_command: &str) -> Option<String> {
    if site.protocol != Protocol::Sftp {
        return None;
    }
    let mut parts = vec!["ssh".to_string(), "-t".to_string()];
    if site.port != 22 {
        parts.push(format!("-p {}", site.port));
    }
    if let Auth::Key { path } = &site.auth {
        parts.push(format!("-i {}", shell_quote(&path.display().to_string())));
    }
    let target = if site.user.is_empty() {
        site.host.clone()
    } else {
        format!("{}@{}", site.user, site.host)
    };
    parts.push(shell_quote(&target));
    parts.push(shell_quote(remote_command));
    Some(parts.join(" "))
}

#[cfg(test)]
mod tests {
    use remote::{Auth, Protocol, Site};

    use super::ssh_command;

    #[test]
    fn builds_ssh_commands() {
        let mut site = Site::new("box", Protocol::Sftp, "1.2.3.4", "deploy");
        site.port = 2222;
        site.auth = Auth::Key {
            path: "/Users/me/.ssh/id_ed25519".into(),
        };
        let command = ssh_command(&site, "tail -n 200 -F '/var/log/app.log'").unwrap();
        assert_eq!(
            command,
            r"ssh -t -p 2222 -i '/Users/me/.ssh/id_ed25519' 'deploy@1.2.3.4' 'tail -n 200 -F '\''/var/log/app.log'\'''"
        );
        site.protocol = Protocol::Ftp;
        assert_eq!(ssh_command(&site, "ls"), None);
    }
}
