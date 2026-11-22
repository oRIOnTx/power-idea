//! Docker integration without the `docker` CLI for everyday work: the Engine
//! API over its unix socket (list, start/stop/restart, events). Layers:
//! `socket` (where is Docker) → `http` → `client` (Engine API) → `project`
//! (which containers belong to the project). Compose itself has no Engine
//! API, so compose commands go through `docker compose` in a terminal.

pub mod client;
pub mod http;
pub mod model;
pub mod project;
pub mod socket;

pub use client::{Action, Docker, Event};
pub use model::{Container, Health, State};
pub use project::{
    ComposeProject, Group, GroupKind, Row, compose_file, groups, has_dockerfile, parse_compose_config, rows,
};
pub use socket::resolve_socket;
