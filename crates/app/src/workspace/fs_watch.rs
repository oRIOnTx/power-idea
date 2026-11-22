//! Follows changes made outside the IDE: refreshes the project tree and the
//! search index, reloads clean editors and warns about edited ones.

use std::time::Duration;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;
use notify::Watcher as _;

use super::Workspace;
use crate::editor::CenterTab;
use crate::inbox::Inbox;
use crate::watcher::Changes;

/// Events are collected this long so a `git checkout` becomes one refresh.
const DEBOUNCE: Duration = Duration::from_millis(400);

impl Workspace {
    pub(super) fn start_watching(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let events: Inbox<notify::Event> = Inbox::default();
        let sender = events.clone();
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            if let Ok(event) = event {
                sender.push(event);
            }
        });
        let mut watcher = match watcher {
            Ok(watcher) => watcher,
            Err(err) => return eprintln!("file watcher unavailable: {err}"),
        };
        if let Err(err) = watcher.watch(&self.root, notify::RecursiveMode::Recursive) {
            return eprintln!("cannot watch {}: {err}", self.root.display());
        }
        self.watcher = Some(watcher);

        cx.spawn_in(window, async move |this, cx| {
            while let Some(mut batch) = events.next_batch().await {
                cx.background_executor().timer(DEBOUNCE).await;
                batch.extend(events.take_all());
                let alive = this.update_in(cx, |ws, window, cx| {
                    let mut changes = Changes::default();
                    for event in batch {
                        changes.add(&ws.root, &ws.project.exclude, event.kind, &event.paths);
                    }
                    if !changes.is_empty() {
                        ws.apply_fs_changes(changes, window, cx);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn apply_fs_changes(&mut self, changes: Changes, window: &mut Window, cx: &mut Context<Self>) {
        if changes.structure {
            self.refresh_tree(cx);
            self.reindex(cx);
        }
        let mut reload = false;
        for tab in &self.tabs {
            let CenterTab::Editor(tab) = tab else { continue };
            if !changes.files.contains(&tab.path) {
                continue;
            }
            let Ok(disk) = std::fs::read_to_string(&tab.path) else {
                continue;
            };
            if disk.as_str() == tab.saved_text.as_ref() {
                continue; // our own save, or no real change
            }
            if tab.is_dirty(cx) {
                let text = format!("{} changed on disk, but it has unsaved changes here", tab.rel);
                window.push_notification(Notification::warning(text), cx);
            } else {
                reload = true;
            }
        }
        if reload {
            self.reload_clean_editors(window, cx);
        }
        cx.notify();
    }
}
