//! Docker tool window (sidebar): the project's containers and compose
//! services with live status from Docker's event stream, start / stop /
//! restart / remove through the Engine API, and logs, shell and compose
//! commands in terminal tabs. Nothing starts until the panel is opened.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::Result;
use docker::{Action, ComposeProject, Container, Docker, Row, State};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;
use run::RunConfig;

use super::Workspace;
use crate::inbox::Inbox;

/// Events often come in bursts (compose up); refresh once per burst.
const REFRESH_DEBOUNCE: Duration = Duration::from_millis(300);

pub(super) enum DockerStatus {
    Connecting,
    Unavailable(String),
    Ready,
}

pub(super) struct DockerState {
    pub(super) status: DockerStatus,
    pub(super) docker: Option<Docker>,
    pub(super) project: Option<ComposeProject>,
    containers: Vec<Container>,
    pub(super) show_all: bool,
    /// Containers with an action in flight (their buttons show progress).
    pub(super) busy: HashSet<String>,
    /// Ends the event-stream thread when the panel state goes away.
    stop_events: Arc<AtomicBool>,
}

impl Drop for DockerState {
    fn drop(&mut self) {
        self.stop_events.store(true, Ordering::Relaxed);
    }
}

impl DockerState {
    pub(super) fn rows(&self, root: &std::path::Path) -> Vec<Row> {
        docker::rows(root, self.project.as_ref(), &self.containers, self.show_all)
    }

    /// (running, total) of the project's rows, for the sidebar badge.
    pub(super) fn counts(&self, root: &std::path::Path) -> (usize, usize) {
        let rows = docker::rows(root, self.project.as_ref(), &self.containers, false);
        (
            rows.iter().filter(|r| r.state() == State::Running).count(),
            rows.len(),
        )
    }
}

/// Everything needed to show the panel: daemon, compose project, containers.
fn connect(root: PathBuf) -> Result<(Docker, Option<ComposeProject>, Vec<Container>)> {
    let home = dirs::home_dir().unwrap_or_default();
    let socket = docker::resolve_socket(std::env::var("DOCKER_HOST").ok().as_deref(), &home)
        .ok_or_else(|| anyhow::anyhow!("Docker is not running (no Docker socket found)"))?;
    let docker = Docker::connect(&socket)?;
    let containers = docker.containers()?;
    // Compose has no Engine API; `config --format json` also spares a YAML parser.
    let project = docker::compose_file(&root).and_then(|_| {
        let output = Command::new("docker")
            .args(["compose", "config", "--format", "json"])
            .current_dir(&root)
            .env("DOCKER_HOST", format!("unix://{}", socket.display()))
            .output()
            .ok()?;
        output
            .status
            .success()
            .then(|| docker::parse_compose_config(&String::from_utf8_lossy(&output.stdout)))
            .flatten()
    });
    Ok((docker, project, containers))
}

impl Workspace {
    /// Opens the panel; the first time it connects and starts following events.
    pub(super) fn ensure_docker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .docker
            .as_ref()
            .is_some_and(|d| !matches!(d.status, DockerStatus::Unavailable(_)))
        {
            return;
        }
        self.docker = Some(DockerState {
            status: DockerStatus::Connecting,
            docker: None,
            project: None,
            containers: Vec::new(),
            show_all: false,
            busy: HashSet::new(),
            stop_events: Arc::new(AtomicBool::new(false)),
        });
        cx.notify();
        let root = self.root.clone();
        cx.spawn_in(window, async move |this, cx| {
            let connected = cx.background_spawn(async move { connect(root) }).await;
            this.update_in(cx, |ws, window, cx| ws.docker_connected(connected, window, cx))
                .ok();
        })
        .detach();
    }

    fn docker_connected(
        &mut self,
        connected: Result<(Docker, Option<ComposeProject>, Vec<Container>)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.docker.as_mut() else { return };
        match connected {
            Ok((docker, project, containers)) => {
                state.status = DockerStatus::Ready;
                state.project = project;
                state.containers = containers;
                state.docker = Some(docker.clone());
                let stop = state.stop_events.clone();
                self.follow_docker_events(docker, stop, window, cx);
            }
            Err(err) => state.status = DockerStatus::Unavailable(format!("{err:#}")),
        }
        cx.notify();
    }

    /// One thread reads `/events`; the UI re-lists containers once per burst.
    fn follow_docker_events(
        &mut self,
        docker: Docker,
        stop: Arc<AtomicBool>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let events: Inbox<()> = Inbox::default();
        let sender = events.clone();
        let reader = docker.clone();
        let _ = std::thread::Builder::new()
            .name("docker-events".into())
            .spawn(move || {
                let _ = reader.follow_events(|_| {
                    sender.push(());
                    !stop.load(Ordering::Relaxed)
                });
                sender.close();
            });
        cx.spawn_in(window, async move |this, cx| {
            while events.next_batch().await.is_some() {
                cx.background_executor().timer(REFRESH_DEBOUNCE).await;
                events.take_all();
                let lister = docker.clone();
                let containers = cx.background_spawn(async move { lister.containers() }).await;
                let alive = this.update(cx, |ws, cx| {
                    if let (Some(state), Ok(containers)) = (ws.docker.as_mut(), containers) {
                        state.containers = containers;
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
            // The stream ended (Docker quit): show the unavailable state.
            this.update(cx, |ws, cx| {
                if let Some(state) = ws.docker.as_mut()
                    && matches!(state.status, DockerStatus::Ready)
                {
                    state.status = DockerStatus::Unavailable("Docker stopped".into());
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn container_action(
        &mut self,
        id: String,
        action: Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.docker.as_mut() else { return };
        let Some(docker) = state.docker.clone() else {
            return;
        };
        state.busy.insert(id.clone());
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let target = id.clone();
            let result = cx
                .background_spawn(async move { docker.act(&target, action) })
                .await;
            this.update_in(cx, |ws, window, cx| {
                if let Some(state) = ws.docker.as_mut() {
                    state.busy.remove(&id);
                }
                if let Err(err) = result {
                    window.push_notification(Notification::error(format!("{err:#}")).title("Docker"), cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn confirm_remove(
        &mut self,
        container: Container,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let workspace = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let (workspace, id) = (workspace.clone(), container.id.clone());
            dialog
                .title(format!("Remove container {}?", container.name))
                .child("The container is stopped and deleted; its image and named volumes stay.")
                .on_ok(move |_, window, cx| {
                    workspace
                        .update(cx, |ws, cx| {
                            ws.container_action(id.clone(), Action::Remove, window, cx);
                        })
                        .ok();
                    true
                })
        });
    }

    /// Runs a docker CLI command (logs, shell, compose) in its own terminal tab.
    pub(super) fn docker_terminal(
        &mut self,
        name: String,
        command: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let socket = self
            .docker
            .as_ref()
            .and_then(|d| d.docker.as_ref())
            .map(|d| d.socket().to_path_buf());
        let mut env = BTreeMap::new();
        if let Some(socket) = socket {
            env.insert("DOCKER_HOST".to_string(), format!("unix://{}", socket.display()));
        }
        let mut config = RunConfig::new(name, command);
        config.env = env;
        self.run_config(config, window, cx);
    }

    pub(super) fn compose(&mut self, args: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.docker_terminal(
            format!("compose {args}"),
            format!("docker compose {args}"),
            window,
            cx,
        );
    }
}
