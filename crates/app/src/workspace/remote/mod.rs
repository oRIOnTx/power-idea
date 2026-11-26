//! Remote Files tool window (⌘7): saved sites, live connections and their lazily loaded
//! folder trees. All network work runs on background threads (`crates/remote`); results
//! come back through `this.update`.

mod conflict_view;
mod dialogs;
mod drag;
mod edit;
pub(crate) mod mcp;
mod ops;
mod panel;
mod rows;
mod site_form;
mod ssh_import;
mod transfers;
mod tree;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::InputState;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;
use remote::listing::SortBy;
use remote::{ConnectOptions, Connection, Credentials, Entry, ErrorKind, RemoteError, Site};

pub(super) use drag::{DragLabel, ProjectDrag, RemoteDrag};
pub(super) use edit::RemoteUpload;

use super::{LeftPanel, Workspace};
use crate::config::Config;

/// How often live connections are checked (no network: the SSH keepalive does that).
const LIVENESS_EVERY: Duration = Duration::from_secs(5);

pub(super) enum Status {
    /// Not connected (never, or after a failure the user has seen).
    Idle,
    Connecting,
    Connected,
    /// The link dropped; the next action reconnects.
    Lost,
    Failed(String),
}

pub(super) enum DirState {
    Loading,
    Loaded(Vec<Entry>),
    Failed(String),
}

/// One site's connection and what the panel shows of it.
pub(super) struct SiteConn {
    pub(super) status: Status,
    pub(super) conn: Option<Arc<Connection>>,
    /// The folder at the top of the tree (the path bar's value).
    pub(super) root: String,
    pub(super) dirs: HashMap<String, DirState>,
    pub(super) expanded: HashSet<String>,
    /// A changed host key the user chose to trust for this session.
    trusted_fingerprint: Option<String>,
    /// Work waiting for the connection (e.g. opening a file after connecting).
    pending: Vec<PendingWork>,
}

/// Gets the connection, or `Cancelled` when connecting failed (already reported).
type PendingWork = Box<
    dyn FnOnce(&mut Workspace, Result<Arc<Connection>, RemoteError>, &mut Window, &mut Context<Workspace>),
>;

impl SiteConn {
    fn idle() -> Self {
        Self {
            status: Status::Idle,
            conn: None,
            root: String::new(),
            dirs: HashMap::new(),
            expanded: HashSet::new(),
            trusted_fingerprint: None,
            pending: Vec::new(),
        }
    }
}

pub(crate) struct RemoteState {
    pub(super) sites: Vec<Site>,
    /// The site shown in the panel.
    pub(super) active: Option<String>,
    pub(super) conns: HashMap<String, SiteConn>,
    pub(super) show_hidden: bool,
    pub(super) sort: SortBy,
    pub(super) filter: Entity<InputState>,
    pub(super) path_input: Entity<InputState>,
    /// Selected row (remote path) in the active site's tree.
    pub(super) selected: Option<String>,
    pub(super) transfers: transfers::Transfers,
    _liveness: Task<()>,
}

impl RemoteState {
    /// Sites with an open connection, for the activity bar badge.
    pub(crate) fn connected_count(&self) -> usize {
        self.conns
            .values()
            .filter(|c| matches!(c.status, Status::Connected))
            .count()
    }

    pub(super) fn site(&self, id: &str) -> Option<&Site> {
        self.sites.iter().find(|s| s.id == id)
    }
}

pub(super) fn sites_path() -> Option<PathBuf> {
    Config::dir().map(|dir| dir.join("remote-sites.toml"))
}

/// Writes an error with its context to the error journal (the red "!" badge).
/// Cancellations and host-key questions are user decisions, not errors.
pub(super) fn journal(err: &RemoteError, context: &str) {
    if matches!(
        err.kind,
        ErrorKind::Cancelled | ErrorKind::HostKeyUnknown(_) | ErrorKind::HostKeyChanged { .. }
    ) {
        return;
    }
    error_journal::report(
        &anyhow::anyhow!("{} ({})", err.user_message(), err.detail),
        context,
    );
}

impl Workspace {
    /// ⌘7: shows the Remote Files panel or hides the sidebar.
    pub(super) fn toggle_remote(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_left_panel(LeftPanel::Remote, cx);
        if self.show_project_view && self.left_panel == LeftPanel::Remote {
            self.ensure_remote(window, cx);
        }
    }

    pub(super) fn ensure_remote(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.remote.is_some() {
            return;
        }
        let sites = match sites_path().map(|path| remote::sites::load(&path)) {
            Some(Ok(sites)) => sites,
            Some(Err(err)) => {
                error_journal::report(&err, "loading remote sites");
                self.notify_error(format!("Cannot read remote sites: {err:#}"), window, cx);
                Vec::new()
            }
            None => Vec::new(),
        };
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter this folder"));
        cx.observe(&filter, |_, _, cx| cx.notify()).detach();
        let path_input = cx.new(|cx| InputState::new(window, cx).placeholder("/path/on/server"));
        let workspace = cx.entity().downgrade();
        crate::style::submit_on_enter(&path_input, window, cx, {
            move |window, cx| {
                workspace
                    .update(cx, |ws, cx| ws.go_to_typed_path(window, cx))
                    .ok();
            }
        });
        let liveness = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(LIVENESS_EVERY).await;
                if this.update(cx, Workspace::check_remote_liveness).is_err() {
                    break;
                }
            }
        });
        self.remote = Some(RemoteState {
            active: sites.first().map(|s| s.id.clone()),
            sites,
            conns: HashMap::new(),
            show_hidden: false,
            sort: SortBy::Name,
            filter,
            path_input,
            selected: None,
            transfers: transfers::Transfers::default(),
            _liveness: liveness,
        });
        cx.notify();
    }

    fn check_remote_liveness(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.remote.as_mut() else { return };
        let mut changed = false;
        for conn in state.conns.values_mut() {
            if let (Status::Connected, Some(live)) = (&conn.status, &conn.conn)
                && !live.is_alive()
            {
                conn.status = Status::Lost;
                changed = true;
            }
        }
        if changed {
            cx.notify();
        }
    }

    /// Persists the site list; failures are reported, the in-memory list stays.
    pub(super) fn save_remote_sites(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(state), Some(path)) = (self.remote.as_ref(), sites_path()) else {
            return;
        };
        if let Err(err) = remote::sites::save(&path, &state.sites) {
            error_journal::report(&err, "saving remote sites");
            self.notify_error(format!("Cannot save remote sites: {err:#}"), window, cx);
        }
    }

    /// Adds or replaces a site (from the site form) and shows it.
    pub(super) fn upsert_remote_site(&mut self, site: Site, window: &mut Window, cx: &mut Context<Self>) {
        self.ensure_remote(window, cx);
        let Some(state) = self.remote.as_mut() else { return };
        let id = site.id.clone();
        match state.sites.iter_mut().find(|s| s.id == id) {
            Some(existing) => {
                // Changed connection settings take effect on the next connect.
                *existing = site;
                state.conns.remove(&id);
            }
            None => state.sites.push(site),
        }
        state.active = Some(id);
        self.save_remote_sites(window, cx);
        cx.notify();
    }

    pub(super) fn delete_remote_site(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.remote.as_mut() else { return };
        state.sites.retain(|s| s.id != id);
        state.conns.remove(id);
        if state.active.as_deref() == Some(id) {
            state.active = state.sites.first().map(|s| s.id.clone());
        }
        remote::secrets::delete_all(id);
        self.save_remote_sites(window, cx);
        cx.notify();
    }

    pub(super) fn select_remote_site(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.remote.as_mut() else { return };
        state.active = Some(id.to_string());
        state.selected = None;
        let root = state.conns.get(id).map_or_default(|c| c.root.clone());
        let path_input = state.path_input.clone();
        path_input.update(cx, |input, cx| input.set_value(root, window, cx));
        cx.notify();
    }

    pub(super) fn connect_remote_site(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.connect_remote_then(id, window, cx, None);
    }

    /// Connects (unless connected or connecting), then runs `then` with the connection.
    pub(super) fn connect_remote_then(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: Option<PendingWork>,
    ) {
        let site = self.remote.as_ref().and_then(|s| s.site(id)).cloned();
        let (Some(state), Some(site)) = (self.remote.as_mut(), site) else {
            // The site was deleted meanwhile: the work learns it will not run.
            if let Some(then) = then {
                then(self, Err(RemoteError::cancelled()), window, cx);
            }
            return;
        };
        let entry = state.conns.entry(id.to_string()).or_insert_with(SiteConn::idle);
        if let (Status::Connected | Status::Lost, Some(conn)) = (&entry.status, entry.conn.clone()) {
            if let Some(then) = then {
                then(self, Ok(conn), window, cx);
            }
            return;
        }
        entry.pending.extend(then);
        if matches!(entry.status, Status::Connecting) {
            return;
        }
        entry.status = Status::Connecting;
        let options = ConnectOptions {
            known_hosts: None,
            trusted_fingerprint: entry.trusted_fingerprint.clone(),
        };
        cx.notify();
        let id = id.to_string();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let conn = remote::open(&site, Credentials::from_keychain(&site), options)?;
                    let start = if site.remote_path.is_empty() {
                        conn.run(|fs| fs.home())?
                    } else {
                        site.remote_path.clone()
                    };
                    Ok::<_, RemoteError>((Arc::new(conn), start))
                })
                .await;
            this.update_in(cx, |ws, window, cx| ws.finish_connect(&id, result, window, cx))
                .ok();
        })
        .detach();
    }

    fn finish_connect(
        &mut self,
        id: &str,
        result: Result<(Arc<Connection>, String), RemoteError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.remote.as_mut() else { return };
        let site_name = state.site(id).map_or_default(|s| s.name.clone());
        let Some(entry) = state.conns.get_mut(id) else {
            return;
        };
        match result {
            Ok((conn, root)) => {
                entry.status = Status::Connected;
                entry.conn = Some(conn.clone());
                entry.root.clone_from(&root);
                let pending = std::mem::take(&mut entry.pending);
                if state.active.as_deref() == Some(id) {
                    let input = state.path_input.clone();
                    input.update(cx, |input, cx| input.set_value(root.clone(), window, cx));
                }
                self.load_remote_dir(id, &root, window, cx);
                for work in pending {
                    work(self, Ok(conn.clone()), window, cx);
                }
            }
            Err(err) => {
                let pending = std::mem::take(&mut entry.pending);
                entry.root.clear();
                match err.kind.clone() {
                    ErrorKind::HostKeyUnknown(key) => {
                        entry.status = Status::Idle;
                        self.confirm_host_key(id, *key, None, window, cx);
                    }
                    ErrorKind::HostKeyChanged { key, line } => {
                        entry.status = Status::Failed("The host key changed".into());
                        self.confirm_host_key(id, *key, Some(line), window, cx);
                    }
                    _ => {
                        entry.status = Status::Failed(err.user_message());
                        journal(&err, &format!("connecting to remote site {site_name}"));
                        window.push_notification(
                            Notification::error(err.user_message())
                                .title(format!("Cannot connect to {site_name}")),
                            cx,
                        );
                    }
                }
                for work in pending {
                    work(self, Err(RemoteError::cancelled()), window, cx);
                }
            }
        }
        cx.notify();
    }

    pub(super) fn disconnect_remote_site(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(state) = self.remote.as_mut() else { return };
        // Dropping the last handle closes the session.
        state.conns.remove(id);
        cx.notify();
    }

    /// After the user accepted a host key: record it (new key) or trust it for this
    /// session (changed key), then connect again.
    pub(super) fn host_key_accepted(
        &mut self,
        id: &str,
        key: remote::HostKey,
        changed: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if changed {
            if let Some(entry) = self.remote.as_mut().and_then(|s| s.conns.get_mut(id)) {
                entry.trusted_fingerprint = Some(key.fingerprint.clone());
            }
        } else if let Err(err) = remote::trust_host_key(&key, &remote::default_known_hosts()) {
            journal(&err, "recording a host key in known_hosts");
            self.notify_error(err.user_message(), window, cx);
            return;
        }
        if let Some(entry) = self.remote.as_mut().and_then(|s| s.conns.get_mut(id)) {
            entry.status = Status::Idle;
        }
        self.connect_remote_site(id, window, cx);
    }

    /// Runs `op` on the site's connection off the UI thread; `done` gets the result.
    /// Failures are explained in a notification and written to the error journal.
    pub(super) fn run_remote<T: Send + 'static>(
        &mut self,
        site_id: &str,
        label: &'static str,
        op: impl FnOnce(&Connection) -> Result<T, RemoteError> + Send + 'static,
        done: impl FnOnce(&mut Self, T, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = site_id.to_string();
        self.run_remote_result(
            site_id,
            op,
            move |ws, result, window, cx| match result {
                Ok(value) => done(ws, value, window, cx),
                Err(err) => ws.remote_failed(&id, label, &err, window, cx),
            },
            window,
            cx,
        );
    }

    /// Like [`Self::run_remote`], but `done` handles errors itself (`Cancelled` = not connected).
    pub(super) fn run_remote_result<T: Send + 'static>(
        &mut self,
        site_id: &str,
        op: impl FnOnce(&Connection) -> Result<T, RemoteError> + Send + 'static,
        done: impl FnOnce(&mut Self, Result<T, RemoteError>, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = site_id.to_string();
        self.connect_remote_then(
            site_id,
            window,
            cx,
            Some(Box::new(move |ws, conn, window, cx| {
                let conn = match conn {
                    Ok(conn) => conn,
                    Err(err) => return done(ws, Err(err), window, cx),
                };
                cx.spawn_in(window, async move |this, cx| {
                    let result = cx.background_spawn(async move { op(&conn) }).await;
                    this.update_in(cx, |ws, window, cx| {
                        if result.is_ok() {
                            ws.mark_remote_alive(&id);
                        }
                        done(ws, result, window, cx);
                    })
                    .ok();
                })
                .detach();
            })),
        );
    }

    fn mark_remote_alive(&mut self, id: &str) {
        if let Some(entry) = self.remote.as_mut().and_then(|s| s.conns.get_mut(id))
            && matches!(entry.status, Status::Lost)
        {
            entry.status = Status::Connected;
        }
    }

    pub(super) fn remote_failed(
        &mut self,
        id: &str,
        label: &str,
        err: &RemoteError,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if err.kind == ErrorKind::Cancelled {
            return;
        }
        if err.is_connection_lost()
            && let Some(entry) = self.remote.as_mut().and_then(|s| s.conns.get_mut(id))
        {
            entry.status = Status::Lost;
        }
        let site = self
            .remote
            .as_ref()
            .and_then(|s| s.site(id))
            .map_or_default(|s| s.name.clone());
        journal(err, &format!("{label} on remote site {site}"));
        window.push_notification(
            Notification::error(err.user_message()).title(format!("{label} failed")),
            cx,
        );
        cx.notify();
    }

    /// The active site's id and connection state, for the panel and actions.
    pub(super) fn active_remote(&self) -> Option<(&Site, Option<&SiteConn>)> {
        let state = self.remote.as_ref()?;
        let site = state.site(state.active.as_deref()?)?;
        Some((site, state.conns.get(&site.id)))
    }
}
