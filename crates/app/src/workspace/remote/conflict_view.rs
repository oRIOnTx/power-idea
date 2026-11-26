//! Resolving an upload conflict on a remote tab: reload the server copy, compare it with
//! the tab's text side by side, or overwrite it.

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::diff::DiffFile;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;
use remote::RemoteError;
use remote::conflict::Snapshot;

use super::Workspace;
use super::dialogs::ConflictChoice;
use crate::editor::{CenterTab, RemoteFile, RemoteSync};

impl Workspace {
    /// The conflict dialog's choices.
    pub(super) fn resolve_remote_conflict(
        &mut self,
        key: &str,
        choice: ConflictChoice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self.remote_tab_ix(key) else { return };
        match choice {
            ConflictChoice::Overwrite => {
                if let Some(file) = self.remote_file_mut(ix) {
                    file.sync = RemoteSync::NotSynced(String::new());
                }
                self.upload_remote_tab(ix, true, window, cx);
            }
            ConflictChoice::Reload => self.reload_remote_tab(key, window, cx),
            ConflictChoice::ShowDiff => self.show_remote_diff(key, window, cx),
        }
    }

    fn remote_file_mut(&mut self, ix: usize) -> Option<&mut RemoteFile> {
        match self.tabs.get_mut(ix) {
            Some(CenterTab::Editor(tab)) => tab.remote.as_mut(),
            _ => None,
        }
    }

    fn remote_file_of(&self, key: &str) -> Option<(String, String)> {
        let ix = self.remote_tab_ix(key)?;
        let file = self.tabs.get(ix)?.editor()?.remote.as_ref()?;
        Some((file.site_id.clone(), file.path.clone()))
    }

    /// Replaces the tab's text with the server's copy (local changes are dropped).
    pub(super) fn reload_remote_tab(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some((site, path)) = self.remote_file_of(key) else {
            return;
        };
        let key = key.to_string();
        self.run_remote(
            &site,
            "Reloading from the server",
            move |conn| conn.run(|fs| Ok((fs.read(&path)?, fs.stat(&path)?))),
            move |ws, (bytes, entry), window, cx| {
                let Some(ix) = ws.remote_tab_ix(&key) else { return };
                let Some(CenterTab::Editor(tab)) = ws.tabs.get_mut(ix) else {
                    return;
                };
                let text = String::from_utf8_lossy(&bytes).into_owned();
                std::fs::write(&tab.path, &bytes).ok();
                tab.state
                    .update(cx, |s, cx| s.set_value(text.clone(), window, cx));
                tab.saved_text = text.into();
                if let Some(file) = tab.remote.as_mut() {
                    file.snapshot = Some(Snapshot::of(&entry));
                    file.sync = RemoteSync::Synced;
                }
                cx.notify();
            },
            window,
            cx,
        );
    }

    /// Side-by-side diff: the server's copy against the tab's text.
    pub(super) fn show_remote_diff(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some((site, path)) = self.remote_file_of(key) else {
            return;
        };
        let Some(local) = self
            .remote_tab_ix(key)
            .and_then(|ix| self.tabs.get(ix))
            .and_then(CenterTab::editor)
            .map(|tab| tab.state.read(cx).value().to_string())
        else {
            return;
        };
        let name = remote::path::file_name(&path).to_string();
        let (diff_key, title) = (format!("remote-diff:{key}"), format!("{name} (server ↔ local)"));
        self.run_remote(
            &site,
            "Comparing with the server",
            move |conn| {
                let server = conn.run(|fs| fs.read(&path))?;
                diff_patch(&name, &String::from_utf8_lossy(&server), &local)
                    .map_err(|e| RemoteError::other(format!("{e:#}")))
            },
            move |ws, patch, window, cx| match DiffFile::parse(&patch) {
                Ok(files) if !files.is_empty() => ws.show_diff_tab(diff_key, title, None, files, window, cx),
                Ok(_) => {
                    window.push_notification(Notification::info("The server copy is the same as yours"), cx);
                }
                Err(err) => ws.notify_error(format!("Cannot show the diff: {err:?}"), window, cx),
            },
            window,
            cx,
        );
    }
}

/// A unified diff of two texts via `git diff --no-index` (headers name the server and local copies).
fn diff_patch(name: &str, server: &str, local: &str) -> Result<String> {
    let dir = std::env::temp_dir().join(format!(
        "power-idea-remote-diff-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    let safe = remote::transfer::safe_local_name(name).unwrap_or("file");
    for (side, text) in [("server", server), ("local", local)] {
        std::fs::create_dir_all(dir.join(side))?;
        std::fs::write(dir.join(side).join(safe), text)?;
    }
    let output = std::process::Command::new("git")
        .current_dir(&dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .args(["diff", "--no-index", "--no-color", "--no-ext-diff", "--"])
        .arg(format!("server/{safe}"))
        .arg(format!("local/{safe}"))
        .output();
    std::fs::remove_dir_all(&dir).ok();
    let output = output?;
    // Exit code 1 means "differences found".
    if !matches!(output.status.code(), Some(0 | 1)) {
        anyhow::bail!(
            "git diff failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::diff_patch;

    #[test]
    fn diffs_two_texts() {
        let patch = diff_patch("nginx.conf", "a\nb\n", "a\nc\n").unwrap();
        assert!(patch.contains("-b"));
        assert!(patch.contains("+c"));
        assert!(diff_patch("x", "same\n", "same\n").unwrap().is_empty());
    }
}
