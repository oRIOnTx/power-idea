//! The error journal in the UI: badge counts kept live from the journal's worker thread,
//! the retention setting, locations of recorded errors, and the debug-only test triggers.

mod view;

use std::path::{Path, PathBuf};

use error_journal::{Event, Retention};
use gpui_kit::*;

use crate::inbox::Inbox;

pub use view::ErrorsView;

/// What the badge and the errors view show; refreshed on every journal event.
#[derive(Debug, Default)]
pub struct ErrorBadge {
    pub unseen: usize,
    pub total: usize,
    /// "Removed 12 old errors" after a shorter retention was picked.
    pub note: Option<String>,
}

impl Global for ErrorBadge {}

/// Subscribes to the journal (started in `main`) and registers the debug triggers.
pub fn init(cx: &mut App) {
    let (unseen, total) = error_journal::counts();
    cx.set_global(ErrorBadge {
        unseen,
        total,
        note: None,
    });
    let inbox: Inbox<Event> = Inbox::default();
    error_journal::subscribe({
        let inbox = inbox.clone();
        move |event| inbox.push(event)
    });
    cx.spawn(async move |cx| {
        while let Some(events) = inbox.next_batch().await {
            cx.update(|cx| apply(&events, cx));
        }
    })
    .detach();
    if cfg!(debug_assertions) {
        register_debug_triggers(cx);
    }
}

fn apply(events: &[Event], cx: &mut App) {
    let (unseen, total) = error_journal::counts();
    cx.update_global::<ErrorBadge, _>(|badge, _| {
        badge.unseen = unseen;
        badge.total = total;
        for event in events {
            if let Event::Pruned(removed) = event {
                badge.note = Some(removed_note(*removed));
            }
        }
    });
    cx.refresh_windows();
}

fn removed_note(removed: usize) -> String {
    match removed {
        0 => "No old errors to remove".into(),
        1 => "Removed 1 old error".into(),
        n => format!("Removed {n} old errors"),
    }
}

/// Saves the setting and prunes at once (on the journal's thread).
pub fn set_retention(retention: Retention, cx: &mut App) {
    crate::settings::update_config(cx, |c| {
        c.error_journal.retention = retention.as_setting().into();
    });
    cx.global_mut::<ErrorBadge>().note = None;
    error_journal::set_retention(retention);
}

/// The badge's accessible label and tooltip.
pub fn badge_label(unseen: usize) -> String {
    if unseen == 1 {
        "1 new error".into()
    } else {
        format!("{unseen} new errors")
    }
}

/// A recorded `file:line[:col]` as a file on disk with a 0-based line and column: `~` is the
/// home dir, relative paths (source paths of this repository) resolve against `root`.
pub fn resolve_location(location: &str, root: &Path, home: Option<&Path>) -> Option<(PathBuf, u32, u32)> {
    let (path, line, column) = error_journal::parse_location(location)?;
    let path = match path.strip_prefix("~/") {
        Some(rest) => home?.join(rest),
        None if Path::new(&path).is_absolute() => PathBuf::from(path),
        None => root.join(path),
    };
    Some((path, line.saturating_sub(1), column.saturating_sub(1)))
}

#[allow(clippy::panic)] // Their whole point is to panic.
fn register_debug_triggers(cx: &mut App) {
    use crate::actions::{TriggerTestCrash, TriggerTestError, TriggerTestPanic};
    cx.on_action(|_: &TriggerTestError, _| {
        let err =
            anyhow::anyhow!("connection refused (os error 61)").context("test error from the debug command");
        error_journal::report(&err, "Debug: Trigger Test Error");
    });
    cx.on_action(|_: &TriggerTestPanic, _| {
        let spawned = std::thread::Builder::new()
            .name("test-panic".into())
            .spawn(|| panic!("test panic from the debug command"));
        if let Err(err) = spawned {
            error_journal::report(&err.into(), "Debug: Trigger Test Panic");
        }
    });
    // A panic inside an AppKit callback aborts the process: the "forced crash" check.
    cx.on_action(|_: &TriggerTestCrash, _| panic!("test crash on the UI thread"));
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{badge_label, removed_note, resolve_location};

    #[test]
    fn resolves_recorded_locations() {
        let root = Path::new("/work/power-idea");
        let home = Some(Path::new("/Users/me"));
        assert_eq!(
            resolve_location("crates/app/src/main.rs:12:5", root, home),
            Some((root.join("crates/app/src/main.rs"), 11, 4))
        );
        assert_eq!(
            resolve_location("~/.cargo/registry/x.rs:3", root, home),
            Some((PathBuf::from("/Users/me/.cargo/registry/x.rs"), 2, 0))
        );
        assert_eq!(
            resolve_location("/rustc/abc/library/core/src/x.rs:1:1", root, None),
            Some((PathBuf::from("/rustc/abc/library/core/src/x.rs"), 0, 0))
        );
        assert_eq!(resolve_location("~/x.rs:1", root, None), None);
        assert_eq!(resolve_location("no location", root, home), None);
    }

    #[test]
    fn labels() {
        assert_eq!(badge_label(1), "1 new error");
        assert_eq!(badge_label(3), "3 new errors");
        assert_eq!(removed_note(12), "Removed 12 old errors");
    }
}
