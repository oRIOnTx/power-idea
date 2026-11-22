//! Containers as the Docker Engine API lists them.

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Running,
    Paused,
    Restarting,
    Exited,
    Created,
    Dead,
    /// A compose service with no container yet.
    NotCreated,
}

impl State {
    fn parse(text: &str) -> Self {
        match text {
            "running" => Self::Running,
            "paused" => Self::Paused,
            "restarting" => Self::Restarting,
            "created" => Self::Created,
            "dead" => Self::Dead,
            _ => Self::Exited,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Restarting => "restarting",
            Self::Exited => "exited",
            Self::Created => "created",
            Self::Dead => "dead",
            Self::NotCreated => "not created",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Healthy,
    Unhealthy,
    Starting,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Port {
    pub private: u16,
    /// Published port on the host.
    pub public: Option<u16>,
    pub protocol: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Container {
    pub id: String,
    pub name: String,
    pub image: String,
    pub state: State,
    /// Docker's human text, e.g. "Up 3 minutes (healthy)".
    pub status: String,
    pub health: Option<Health>,
    pub ports: Vec<Port>,
    /// Compose labels (empty for plain containers).
    pub project: String,
    pub service: String,
    pub working_dir: String,
}

impl Container {
    pub fn short_id(&self) -> &str {
        self.id.get(..12).unwrap_or(&self.id)
    }

    /// Distinct published host ports, for `localhost:PORT` links.
    pub fn public_ports(&self) -> Vec<u16> {
        let mut ports: Vec<u16> = self.ports.iter().filter_map(|p| p.public).collect();
        ports.sort_unstable();
        ports.dedup();
        ports
    }
}

fn label<'a>(labels: &'a Value, key: &str) -> &'a str {
    labels.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// One entry of `GET /containers/json`.
pub fn parse_container(value: &Value) -> Option<Container> {
    let labels = value.get("Labels").cloned().unwrap_or(Value::Null);
    let status = value
        .get("Status")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let health = if status.contains("(unhealthy)") {
        Some(Health::Unhealthy)
    } else if status.contains("(healthy)") {
        Some(Health::Healthy)
    } else if status.contains("(health: starting)") {
        Some(Health::Starting)
    } else {
        None
    };
    let ports = value
        .get("Ports")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|p| {
            Some(Port {
                private: p.get("PrivatePort")?.as_u64()? as u16,
                public: p.get("PublicPort").and_then(Value::as_u64).map(|n| n as u16),
                protocol: p.get("Type").and_then(Value::as_str).unwrap_or("tcp").to_string(),
            })
        })
        .collect();
    Some(Container {
        id: value.get("Id")?.as_str()?.to_string(),
        name: value
            .pointer("/Names/0")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim_start_matches('/')
            .to_string(),
        image: value
            .get("Image")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        state: State::parse(value.get("State").and_then(Value::as_str).unwrap_or_default()),
        status,
        health,
        ports,
        project: label(&labels, "com.docker.compose.project").to_string(),
        service: label(&labels, "com.docker.compose.service").to_string(),
        working_dir: label(&labels, "com.docker.compose.project.working_dir").to_string(),
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Health, State, parse_container};

    #[test]
    fn parses_list_entries() {
        let c = parse_container(&json!({
            "Id": "0123456789abcdef0123", "Names": ["/shop-web-1"], "Image": "shop-web",
            "State": "running", "Status": "Up 3 minutes (healthy)",
            "Ports": [{"PrivatePort": 3000, "PublicPort": 3000, "Type": "tcp", "IP": "0.0.0.0"},
                      {"PrivatePort": 3000, "PublicPort": 3000, "Type": "tcp", "IP": "::"},
                      {"PrivatePort": 9229, "Type": "tcp"}],
            "Labels": {"com.docker.compose.project": "shop", "com.docker.compose.service": "web",
                       "com.docker.compose.project.working_dir": "/p/shop"}
        }))
        .unwrap();
        assert_eq!((c.name.as_str(), c.short_id()), ("shop-web-1", "0123456789ab"));
        assert_eq!((c.state, c.health), (State::Running, Some(Health::Healthy)));
        assert_eq!(c.public_ports(), [3000]);
        assert_eq!((c.project.as_str(), c.service.as_str()), ("shop", "web"));

        let exited =
            parse_container(&json!({"Id": "x", "State": "exited", "Status": "Exited (1) 2 hours ago"}))
                .unwrap();
        assert_eq!((exited.state, exited.health), (State::Exited, None));
        assert!(exited.service.is_empty());
    }
}
