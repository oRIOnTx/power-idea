//! Docker Engine API client: list containers, act on them, follow events.

use std::io::BufRead;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde_json::Value;

use super::http;
use super::model::{Container, parse_container};

/// What a container button does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Start,
    Stop,
    Restart,
    Pause,
    Unpause,
    Remove,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Self::Start => "Start",
            Self::Stop => "Stop",
            Self::Restart => "Restart",
            Self::Pause => "Pause",
            Self::Unpause => "Unpause",
            Self::Remove => "Remove",
        }
    }

    fn request(self, id: &str) -> (&'static str, String) {
        match self {
            Self::Remove => ("DELETE", format!("/containers/{id}?force=1")),
            other => {
                let verb = match other {
                    Self::Start => "start",
                    Self::Stop => "stop",
                    Self::Restart => "restart",
                    Self::Pause => "pause",
                    _ => "unpause",
                };
                ("POST", format!("/containers/{id}/{verb}"))
            }
        }
    }
}

/// A container event from `/events`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub id: String,
    /// `start`, `die`, `health_status: healthy`, …
    pub action: String,
}

pub fn parse_event(line: &str) -> Option<Event> {
    let value: Value = serde_json::from_str(line).ok()?;
    Some(Event {
        id: value
            .pointer("/Actor/ID")
            .or_else(|| value.get("id"))?
            .as_str()?
            .to_string(),
        action: value
            .get("Action")
            .or_else(|| value.get("status"))?
            .as_str()?
            .to_string(),
    })
}

#[derive(Debug, Clone)]
pub struct Docker {
    socket: PathBuf,
}

impl Docker {
    /// Connects to `socket` and checks the daemon answers.
    pub fn connect(socket: &Path) -> Result<Self> {
        if !socket.exists() {
            bail!("Docker is not running ({} not found)", socket.display());
        }
        let docker = Self {
            socket: socket.to_path_buf(),
        };
        let ping =
            http::request(&docker.socket, "GET", "/_ping", None).context("Docker is not responding")?;
        if ping.status != 200 {
            bail!("Docker is not ready (HTTP {})", ping.status);
        }
        Ok(docker)
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// All containers, stopped ones included.
    pub fn containers(&self) -> Result<Vec<Container>> {
        let list = http::request(&self.socket, "GET", "/containers/json?all=1", None)?.json()?;
        Ok(list
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(parse_container)
            .collect())
    }

    pub fn act(&self, id: &str, action: Action) -> Result<()> {
        let (method, path) = action.request(id);
        http::request(&self.socket, method, &path, None)?
            .ok()
            .with_context(|| format!("{} {}", action.label(), id.get(..12).unwrap_or(id)))
    }

    /// Calls `on_event` for each container event until it returns false or
    /// the connection drops. Blocks: run it on a background thread.
    pub fn follow_events(&self, mut on_event: impl FnMut(Event) -> bool) -> Result<()> {
        // filters={"type":["container"]}
        let path = "/events?filters=%7B%22type%22%3A%5B%22container%22%5D%7D";
        for line in http::stream(&self.socket, path)?.lines() {
            if let Some(event) = parse_event(&line?)
                && !on_event(event)
            {
                break;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Action, Docker, parse_event};
    use crate::http::tests::fake_docker;

    #[test]
    fn lists_and_acts_on_containers() {
        let (socket, server) = fake_docker("ping", "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nOK");
        let docker = Docker::connect(&socket).unwrap();
        server.join().unwrap();

        let (_, server) = fake_docker(
            "ping",
            "HTTP/1.1 200 OK\r\nContent-Length: 62\r\n\r\n[{\"Id\":\"abc\",\"Names\":[\"/db\"],\"State\":\"running\",\"Status\":\"Up\"}]",
        );
        let containers = docker.containers().unwrap();
        assert_eq!(containers[0].name, "db");
        assert!(server.join().unwrap().starts_with("GET /containers/json?all=1 "));

        let (_, server) = fake_docker("ping", "HTTP/1.1 304 Not Modified\r\nContent-Length: 0\r\n\r\n");
        docker.act("abc", Action::Start).unwrap();
        assert!(server.join().unwrap().starts_with("POST /containers/abc/start "));

        let (_, server) = fake_docker("ping", "HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n");
        docker.act("abc", Action::Remove).unwrap();
        assert!(
            server
                .join()
                .unwrap()
                .starts_with("DELETE /containers/abc?force=1 ")
        );
    }

    #[test]
    fn parses_events() {
        let event = parse_event(
            r#"{"Type":"container","Action":"die","Actor":{"ID":"abc","Attributes":{"name":"db"}}}"#,
        )
        .unwrap();
        assert_eq!((event.id.as_str(), event.action.as_str()), ("abc", "die"));
        assert!(parse_event("not json").is_none());
    }
}
