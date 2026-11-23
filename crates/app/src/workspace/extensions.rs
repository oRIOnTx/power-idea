//! Extension processes of this project window: starting them (eagerly when they listen to
//! events, else on their first command), running commands, sending events and routing
//! their requests (`extension_requests`) back to the UI thread.

use std::collections::HashMap;
use std::sync::Arc;
use std::thread;

use ::extensions::{Extension, Host, Incoming};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;

use super::Workspace;
use crate::extension_registry;
use crate::inbox::Inbox;

/// Per-window extension state.
#[derive(Default)]
pub(super) struct ExtensionHosts {
    slots: HashMap<String, Slot>,
    inbox: Inbox<Message>,
    /// Identifies one start of a process, so messages of a replaced one are ignored.
    next_start: u64,
}

impl Drop for ExtensionHosts {
    fn drop(&mut self) {
        // Ends the message loop of the closed window.
        self.inbox.close();
    }
}

enum Slot {
    Starting {
        start: u64,
        queued: Vec<String>,
    },
    Running {
        start: u64,
        host: Arc<Host>,
    },
    /// Crashed or failed to start; the next command tries again.
    Stopped,
}

impl Slot {
    fn start(&self) -> Option<u64> {
        match self {
            Self::Starting { start, .. } | Self::Running { start, .. } => Some(*start),
            Self::Stopped => None,
        }
    }
}

struct Message {
    extension: Arc<Extension>,
    start: u64,
    kind: MessageKind,
}

enum MessageKind {
    Started(Result<Arc<Host>, String>),
    Incoming(Incoming),
    CommandFailed { command: String, error: String },
}

impl Workspace {
    /// Starts the message loop and the extensions that listen to events.
    pub(super) fn start_extensions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let inbox = self.extensions.inbox.clone();
        let handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            while let Some(batch) = inbox.next_batch().await {
                let alive = cx.update(|cx| {
                    handle.update(cx, |_, window, cx| {
                        this.update(cx, |ws, cx| {
                            for message in batch {
                                ws.handle_extension_message(message, window, cx);
                            }
                        })
                    })
                });
                if !matches!(alive, Ok(Ok(()))) {
                    break;
                }
            }
        })
        .detach();
        self.start_eager_extensions(window, cx);
    }

    /// After Reload Extensions: stops every process and starts the eager ones again.
    pub(crate) fn restart_extensions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.light {
            return;
        }
        self.extensions.slots.clear();
        self.start_eager_extensions(window, cx);
    }

    fn start_eager_extensions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let errors = extension_registry::take_errors(cx);
        if !errors.is_empty() {
            window.push_notification(extension_registry::errors_notification(&errors), cx);
        }
        for extension in extension_registry::installed(cx) {
            if extension.starts_eagerly() {
                self.start_extension(extension, Vec::new());
            }
        }
    }

    /// Spawns the process on its own thread (the handshake blocks); `queued` commands run
    /// once it is up.
    fn start_extension(&mut self, extension: Arc<Extension>, queued: Vec<String>) {
        let start = self.extensions.next_start;
        self.extensions.next_start += 1;
        self.extensions
            .slots
            .insert(extension.id().to_string(), Slot::Starting { start, queued });

        let inbox = self.extensions.inbox.clone();
        let root = self.root.clone();
        let _ = thread::Builder::new()
            .name(format!("ext-start-{}", extension.id()))
            .spawn(move || {
                let handler: ::extensions::Handler = {
                    let (inbox, extension) = (inbox.clone(), extension.clone());
                    Arc::new(move |incoming: Incoming| {
                        inbox.push(Message {
                            extension: extension.clone(),
                            start,
                            kind: MessageKind::Incoming(incoming),
                        });
                    })
                };
                let log = extension_registry::log_path(extension.id());
                let result =
                    Host::start(&extension, &root, log.as_deref(), handler).map_err(|e| format!("{e:#}"));
                inbox.push(Message {
                    extension,
                    start,
                    kind: MessageKind::Started(result),
                });
            });
    }

    /// Runs extension command `id` (Find Action or its key binding).
    pub(super) fn run_extension_command(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.light {
            let note = Notification::warning("Extensions run in project windows, not in LightEdit.");
            return window.push_notification(note, cx);
        }
        let Some(extension) = extension_registry::owner_of(id, cx) else {
            let note = format!("No installed extension provides the command {id}. Try Reload Extensions.");
            return window.push_notification(Notification::error(note), cx);
        };
        match self.extensions.slots.get_mut(extension.id()) {
            Some(Slot::Running { host, start }) => {
                let (host, start) = (host.clone(), *start);
                self.execute_extension_command(extension, start, host, id.to_string());
            }
            Some(Slot::Starting { queued, .. }) => queued.push(id.to_string()),
            Some(Slot::Stopped) | None => self.start_extension(extension, vec![id.to_string()]),
        }
    }

    /// Waits for the answer on its own thread: commands may block on the user for minutes.
    fn execute_extension_command(
        &self,
        extension: Arc<Extension>,
        start: u64,
        host: Arc<Host>,
        command: String,
    ) {
        let inbox = self.extensions.inbox.clone();
        let _ = thread::Builder::new().name("ext-command".into()).spawn(move || {
            if let Err(err) = host.execute(&command) {
                inbox.push(Message {
                    extension,
                    start,
                    kind: MessageKind::CommandFailed {
                        command,
                        error: format!("{err:#}"),
                    },
                });
            }
        });
    }

    /// Tells running extensions that subscribed to `kind` about `rel` (a project path).
    pub(super) fn send_extension_event(&self, kind: &str, rel: &str, cx: &App) {
        if self.extensions.slots.is_empty() {
            return;
        }
        for extension in extension_registry::installed(cx) {
            if let Some(Slot::Running { host, .. }) = self.extensions.slots.get(extension.id())
                && extension.subscribes_to(kind)
            {
                host.send_event(kind, rel);
            }
        }
    }

    /// `file_changed` for the editor `state` (an edit in its buffer).
    pub(super) fn send_file_changed(&self, state: &Entity<component::input::EditorState>, cx: &App) {
        if self.extensions.slots.is_empty() {
            return;
        }
        let tab = self
            .tabs
            .iter()
            .filter_map(crate::editor::CenterTab::editor)
            .find(|t| &t.state == state);
        if let Some(tab) = tab {
            self.send_extension_event("file_changed", &tab.rel, cx);
        }
    }

    fn handle_extension_message(&mut self, message: Message, window: &mut Window, cx: &mut Context<Self>) {
        let Message {
            extension,
            start,
            kind,
        } = message;
        let id = extension.id().to_string();
        // A message from a process that was since restarted or stopped.
        if self.extensions.slots.get(&id).and_then(Slot::start) != Some(start) {
            return;
        }
        match kind {
            MessageKind::Started(Ok(host)) => {
                let queued = match self.extensions.slots.remove(&id) {
                    Some(Slot::Starting { queued, .. }) => queued,
                    _ => Vec::new(),
                };
                self.extensions.slots.insert(
                    id,
                    Slot::Running {
                        start,
                        host: host.clone(),
                    },
                );
                for command in queued {
                    self.execute_extension_command(extension.clone(), start, host.clone(), command);
                }
            }
            MessageKind::Started(Err(error)) => {
                self.extensions.slots.insert(id, Slot::Stopped);
                let note = Notification::error(error)
                    .title(format!("Extension {} failed to start", extension.name()));
                window.push_notification(note, cx);
            }
            MessageKind::CommandFailed { command, error } => {
                let note =
                    Notification::error(error).title(format!("{}: {command} failed", extension.name()));
                window.push_notification(note, cx);
            }
            MessageKind::Incoming(Incoming::Closed) => {
                let reason = match self.extensions.slots.insert(id, Slot::Stopped) {
                    Some(Slot::Running { host, .. }) => host.last_error(),
                    _ => None,
                };
                let note = Notification::warning(reason.unwrap_or_else(|| "The process exited.".into()))
                    .title(format!("Extension {} stopped", extension.name()));
                window.push_notification(note, cx);
            }
            MessageKind::Incoming(Incoming::Request {
                id: request,
                method,
                params,
            }) => {
                if let Some(Slot::Running { host, .. }) = self.extensions.slots.get(&id) {
                    let host = host.clone();
                    self.handle_extension_request(&extension, host, request, &method, params, window, cx);
                }
            }
            // Extensions have no notifications for the IDE yet.
            MessageKind::Incoming(Incoming::Notification { .. }) => {}
        }
    }
}
