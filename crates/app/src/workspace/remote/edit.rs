//! Remote files in editor tabs: download to the cache and open, upload on save after a
//! conflict check (overwrite / reload / diff when the server copy changed).

use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::component::notification::Notification;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, WindowExt as _, h_flex};
use gpui_kit::*;
use remote::conflict::{self, Conflict, Snapshot};
use remote::{Connection, ErrorKind, RemoteError};

use super::Workspace;
use crate::editor::{self, CenterTab, RemoteFile, RemoteSync};
use crate::preview;

/// Bigger files are better downloaded than edited in place.
const MAX_EDIT_BYTES: u64 = 20 * 1024 * 1024;

pub(super) enum UploadOutcome {
    Uploaded(Snapshot),
    Conflict(Conflict),
}

/// Uploads `bytes` to `path` unless the server copy changed since `snapshot` (or `force`).
fn upload_checked(
    conn: &Connection,
    path: &str,
    snapshot: Option<Snapshot>,
    bytes: &[u8],
    force: bool,
) -> Result<UploadOutcome, RemoteError> {
    conn.run(|fs| {
        if !force {
            let current = match fs.stat(path) {
                Ok(entry) => Some(Snapshot::of(&entry)),
                Err(err) if err.kind == ErrorKind::NotFound => None,
                Err(err) => return Err(err),
            };
            let found = conflict::check(snapshot, current);
            if found != Conflict::None {
                return Ok(UploadOutcome::Conflict(found));
            }
        }
        fs.write(path, bytes)?;
        Ok(UploadOutcome::Uploaded(Snapshot::of(&fs.stat(path)?)))
    })
}

/// What a remote tab needs uploaded, captured on the UI thread.
pub(in crate::workspace) struct RemoteUpload {
    pub(in crate::workspace) conn: Option<Arc<Connection>>,
    pub(in crate::workspace) site_name: String,
    pub(in crate::workspace) path: String,
    pub(in crate::workspace) snapshot: Option<Snapshot>,
}

impl RemoteUpload {
    /// The checked upload used when closing the window (no dialogs: a conflict stops the close).
    pub(in crate::workspace) fn run(&self, bytes: &[u8]) -> Result<Option<Snapshot>> {
        let Some(conn) = &self.conn else {
            anyhow::bail!(
                "not connected to {}: connect or discard the changes",
                self.site_name
            );
        };
        match upload_checked(conn, &self.path, self.snapshot, bytes, false)? {
            UploadOutcome::Uploaded(snapshot) => Ok(Some(snapshot)),
            UploadOutcome::Conflict(_) => anyhow::bail!(
                "{} changed on {} since it was opened; save it from its tab to resolve",
                self.path,
                self.site_name
            ),
        }
    }
}

impl Workspace {
    /// `site:` before a remote tab's file name, in the site's label color.
    pub(in crate::workspace) fn remote_tab_prefix(&self, file: &RemoteFile, cx: &App) -> AnyElement {
        let color = self
            .remote
            .as_ref()
            .and_then(|s| s.site(&file.site_id))
            .and_then(|site| super::site_form::site_color(site.color.as_deref(), cx))
            .unwrap_or(cx.theme().muted_foreground);
        h_flex()
            .gap_1()
            .text_color(color)
            .child(crate::style::lucide("server").with_size(px(12.)))
            .child(div().text_xs().child(format!("{}:", file.site_name)))
            .into_any_element()
    }

    /// Uploading spinner, or a red mark when the server does not have the last save.
    pub(in crate::workspace) fn remote_sync_mark(
        ix: usize,
        file: &RemoteFile,
        cx: &App,
    ) -> Option<AnyElement> {
        let theme = cx.theme();
        let (icon, color, text) = match &file.sync {
            RemoteSync::Synced => return None,
            RemoteSync::Uploading => ("loader-circle", theme.muted_foreground, "Uploading…".to_string()),
            RemoteSync::NotSynced(why) => (
                "cloud-off",
                theme.danger,
                if why.is_empty() {
                    "Not synced with the server".to_string()
                } else {
                    format!("Not synced: {why}. Save again to retry.")
                },
            ),
        };
        Some(
            div()
                .id(("remote-sync", ix))
                .child(crate::style::lucide(icon).with_size(px(12.)).text_color(color))
                .tooltip(move |window, cx| Tooltip::new(text.clone()).build(window, cx))
                .into_any_element(),
        )
    }

    /// What closing the window must upload for `tab` (`None` for local files).
    pub(in crate::workspace) fn remote_upload_for(&self, tab: &editor::EditorTab) -> Option<RemoteUpload> {
        let file = tab.remote.as_ref()?;
        let conn = self
            .remote
            .as_ref()
            .and_then(|s| s.conns.get(&file.site_id))
            .and_then(|c| c.conn.clone());
        Some(RemoteUpload {
            conn,
            site_name: file.site_name.clone(),
            path: file.path.clone(),
            snapshot: file.snapshot,
        })
    }

    pub(super) fn remote_tab_ix(&self, rel: &str) -> Option<usize> {
        self.tabs
            .iter()
            .position(|t| t.editor().is_some_and(|e| e.remote.is_some() && e.rel == rel))
    }

    fn cache_path(site_id: &str, path: &str) -> Option<PathBuf> {
        remote::cache::default_root().map(|root| remote::cache::local_path(&root, site_id, path))
    }

    /// Opens a server file in an editor tab (an image in the image viewer).
    pub(super) fn open_remote_file(
        &mut self,
        site_id: &str,
        path: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(site) = self.remote.as_ref().and_then(|s| s.site(site_id)).cloned() else {
            return;
        };
        let rel = editor::remote_rel(&site.name, path);
        if let Some(ix) = self.remote_tab_ix(&rel) {
            self.record_location_if_leaving(&rel, false, cx);
            return self.activate_tab(ix, window, cx);
        }
        let Some(cache) = Self::cache_path(site_id, path) else {
            return self.notify_error("No cache folder for remote files".into(), window, cx);
        };
        let (remote_path, site_key) = (path.to_string(), site_id.to_string());
        let target = cache.clone();
        self.run_remote(
            site_id,
            "Opening a remote file",
            move |conn| {
                conn.run(|fs| {
                    let entry = fs.stat(&remote_path)?;
                    if entry.is_dir() {
                        return Err(RemoteError::other(format!("{remote_path} is a folder")));
                    }
                    if entry.size > MAX_EDIT_BYTES {
                        return Err(RemoteError::other(format!(
                            "{remote_path} is {}; use Download to… for large files",
                            remote::listing::format_size(entry.size)
                        )));
                    }
                    let bytes = fs.read(&remote_path)?;
                    if let Some(dir) = target.parent() {
                        std::fs::create_dir_all(dir)?;
                    }
                    std::fs::write(&target, &bytes)?;
                    Ok((entry, bytes))
                })
            },
            move |ws, (entry, bytes), window, cx| {
                // Opened twice quickly: the first download already made the tab.
                if let Some(ix) = ws.remote_tab_ix(&rel) {
                    return ws.activate_tab(ix, window, cx);
                }
                if preview::is_image(&cache) {
                    return ws.open_image(&rel, cache, window, cx);
                }
                if bytes.contains(&0) {
                    return ws.notify_error(
                        format!("{} looks like a binary file; use Download to…", entry.name),
                        window,
                        cx,
                    );
                }
                let text = String::from_utf8_lossy(&bytes).into_owned();
                let file = RemoteFile {
                    site_id: site_key,
                    site_name: site.name.clone(),
                    path: entry.path.clone(),
                    snapshot: Some(Snapshot::of(&entry)),
                    sync: RemoteSync::Synced,
                };
                let ix = ws.push_editor_tab(cache, rel.clone(), text, Some(file), window, cx);
                ws.record_location_if_leaving(&rel, false, cx);
                ws.activate_tab(ix, window, cx);
            },
            window,
            cx,
        );
    }

    /// `site name:/path` ids (Back, Recent Files): activates or reopens the remote file.
    pub(in crate::workspace) fn open_remote_rel(
        &mut self,
        rel: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if let Some(ix) = self.remote_tab_ix(rel) {
            self.activate_tab(ix, window, cx);
            return true;
        }
        let Some((name, path)) = rel.split_once(':') else {
            return false;
        };
        if !path.starts_with('/') || self.root.join(rel).exists() {
            return false;
        }
        self.ensure_remote(window, cx);
        let Some(site) = self
            .remote
            .as_ref()
            .and_then(|s| s.sites.iter().find(|site| site.name == name))
            .map(|s| s.id.clone())
        else {
            return false;
        };
        self.open_remote_file(&site, path, window, cx);
        true
    }

    /// ⌘S on a remote tab: keep a local copy, then upload in the background.
    pub(in crate::workspace) fn save_remote_tab(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(CenterTab::Editor(tab)) = self.tabs.get(ix) else {
            return true;
        };
        let text = tab.state.read(cx).value();
        if let Err(err) = std::fs::write(&tab.path, text.as_bytes()) {
            let message = format!("Cannot keep a local copy of {}: {err}", tab.rel);
            self.notify_error(message, window, cx);
        }
        self.upload_remote_tab(ix, false, window, cx);
        true
    }

    pub(super) fn upload_remote_tab(
        &mut self,
        ix: usize,
        force: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(CenterTab::Editor(tab)) = self.tabs.get_mut(ix) else {
            return;
        };
        let text = tab.state.read(cx).value();
        let key = tab.rel.clone();
        let Some(file) = tab.remote.as_mut() else { return };
        if file.sync == RemoteSync::Uploading {
            return;
        }
        file.sync = RemoteSync::Uploading;
        let (site, path, snapshot) = (file.site_id.clone(), file.path.clone(), file.snapshot);
        cx.notify();
        let bytes = text.clone();
        self.run_remote_result(
            &site,
            move |conn| upload_checked(conn, &path, snapshot, bytes.as_bytes(), force),
            move |ws, result, window, cx| ws.finish_upload(&key, text, result, window, cx),
            window,
            cx,
        );
    }

    fn finish_upload(
        &mut self,
        key: &str,
        text: SharedString,
        result: Result<UploadOutcome, RemoteError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self.remote_tab_ix(key) else { return };
        let Some(CenterTab::Editor(tab)) = self.tabs.get_mut(ix) else {
            return;
        };
        let Some(file) = tab.remote.as_mut() else { return };
        let (site_id, name) = (
            file.site_id.clone(),
            remote::path::file_name(&file.path).to_string(),
        );
        let site_name = file.site_name.clone();
        match result {
            Ok(UploadOutcome::Uploaded(snapshot)) => {
                file.snapshot = Some(snapshot);
                file.sync = RemoteSync::Synced;
                // The tab counts as saved only once the server has the text.
                tab.saved_text = text;
                let dir = remote::path::parent(&file.path);
                window.push_notification(
                    Notification::success(format!("Uploaded {name} to {site_name}")),
                    cx,
                );
                self.refresh_remote_dir(&site_id, &dir, window, cx);
            }
            Ok(UploadOutcome::Conflict(found)) => {
                file.sync = RemoteSync::NotSynced("changed on the server".into());
                self.ask_remote_conflict(key, &name, &site_name, found, window, cx);
            }
            Err(err) => {
                let not_connected = err.kind == ErrorKind::Cancelled;
                file.sync = RemoteSync::NotSynced(if not_connected {
                    "not connected".into()
                } else {
                    err.user_message()
                });
                if not_connected {
                    window.push_notification(
                        Notification::warning(format!(
                            "{name} was not uploaded: not connected to {site_name}. Your changes are kept."
                        )),
                        cx,
                    );
                } else {
                    self.remote_failed(&site_id, "Uploading", &err, window, cx);
                }
            }
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use remote::RemoteFs;
    use remote::conflict::{Conflict, Snapshot};

    use super::{Connection, UploadOutcome, upload_checked};

    /// A tiny `RemoteFs` with one file, to drive `upload_checked`.
    struct OneFile(std::sync::Mutex<(Vec<u8>, u64)>);

    impl RemoteFs for OneFile {
        fn home(&self) -> remote::Result<String> {
            Ok("/".into())
        }
        fn list(&self, _: &str) -> remote::Result<Vec<remote::Entry>> {
            Ok(Vec::new())
        }
        fn stat(&self, path: &str) -> remote::Result<remote::Entry> {
            let (bytes, mtime) = self.0.lock().unwrap().clone();
            Ok(remote::Entry {
                name: "f".into(),
                path: path.into(),
                kind: remote::EntryKind::File,
                symlink: false,
                size: bytes.len() as u64,
                modified: Some(mtime),
                permissions: None,
                owner: None,
                group: None,
            })
        }
        fn download(
            &self,
            _: &str,
            out: &mut dyn std::io::Write,
            _: remote::Progress<'_>,
        ) -> remote::Result<u64> {
            let bytes = self.0.lock().unwrap().0.clone();
            out.write_all(&bytes)?;
            Ok(bytes.len() as u64)
        }
        fn upload(
            &self,
            _: &str,
            input: &mut dyn std::io::Read,
            _: remote::Progress<'_>,
        ) -> remote::Result<u64> {
            let mut bytes = Vec::new();
            input.read_to_end(&mut bytes)?;
            let mut state = self.0.lock().unwrap();
            *state = (bytes, state.1 + 1);
            Ok(state.0.len() as u64)
        }
        fn rename(&self, _: &str, _: &str) -> remote::Result<()> {
            Ok(())
        }
        fn remove_file(&self, _: &str) -> remote::Result<()> {
            Ok(())
        }
        fn remove_dir(&self, _: &str) -> remote::Result<()> {
            Ok(())
        }
        fn mkdir(&self, _: &str) -> remote::Result<()> {
            Ok(())
        }
        fn chmod(&self, _: &str, _: u32) -> remote::Result<()> {
            Ok(())
        }
        fn is_alive(&self) -> bool {
            true
        }
    }

    #[test]
    fn upload_stops_on_a_server_side_change() {
        let fs = Arc::new(OneFile(std::sync::Mutex::new((b"v1".to_vec(), 10))));
        let shared = fs.clone();
        let conn = Connection::open(Box::new(move || Ok(shared.clone() as Arc<dyn RemoteFs>))).unwrap();
        let opened = Some(Snapshot::of(&fs.stat("/f").unwrap()));

        // Unchanged on the server: uploads and returns the new snapshot.
        let Ok(UploadOutcome::Uploaded(after)) = upload_checked(&conn, "/f", opened, b"v2!", false) else {
            panic!("expected an upload");
        };
        assert_eq!(after.size, 3);

        // Someone else saved meanwhile: our old snapshot no longer matches.
        fs.upload("/f", &mut &b"theirs"[..], &mut |_| true).unwrap();
        assert!(matches!(
            upload_checked(&conn, "/f", Some(after), b"mine", false),
            Ok(UploadOutcome::Conflict(Conflict::Changed))
        ));
        // Overwrite wins.
        assert!(matches!(
            upload_checked(&conn, "/f", Some(after), b"mine", true),
            Ok(UploadOutcome::Uploaded(_))
        ));
        assert_eq!(fs.read("/f").unwrap(), b"mine");
    }
}
