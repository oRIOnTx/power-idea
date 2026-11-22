//! Finds the Docker Engine socket: `DOCKER_HOST`, then the active `docker
//! context`, then the usual sockets of Docker Desktop, OrbStack and Colima.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

fn unix_path(host: &str) -> Option<PathBuf> {
    host.strip_prefix("unix://").map(PathBuf::from)
}

/// The socket of the current `docker context` (only `currentContext` is read
/// from `config.json`, never the stored credentials).
fn context_socket(home: &Path) -> Option<PathBuf> {
    let config: Value =
        serde_json::from_str(&fs::read_to_string(home.join(".docker/config.json")).ok()?).ok()?;
    let name = config.get("currentContext")?.as_str()?;
    if name.is_empty() || name == "default" {
        return None;
    }
    // Context folders are named by a hash of the name; read each meta.json instead of hashing.
    fs::read_dir(home.join(".docker/contexts/meta"))
        .ok()?
        .flatten()
        .find_map(|dir| {
            let meta: Value =
                serde_json::from_str(&fs::read_to_string(dir.path().join("meta.json")).ok()?).ok()?;
            (meta.get("Name")?.as_str()? == name)
                .then(|| {
                    meta.pointer("/Endpoints/docker/Host")?
                        .as_str()
                        .and_then(unix_path)
                })
                .flatten()
        })
}

/// The first socket that exists, honoring an explicit `DOCKER_HOST`.
pub fn resolve_socket(docker_host: Option<&str>, home: &Path) -> Option<PathBuf> {
    if let Some(host) = docker_host.filter(|h| !h.is_empty()) {
        // tcp:// and ssh:// hosts are not supported yet.
        return unix_path(host);
    }
    if let Some(socket) = context_socket(home) {
        return Some(socket);
    }
    [
        PathBuf::from("/var/run/docker.sock"),
        home.join(".docker/run/docker.sock"),
        home.join(".orbstack/run/docker.sock"),
        home.join(".colima/default/docker.sock"),
    ]
    .into_iter()
    .find(|p| p.exists())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::resolve_socket;

    #[test]
    fn resolves_in_order() {
        let home = std::env::temp_dir().join(format!("pi-docker-home-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(home.join(".docker/contexts/meta/abc")).unwrap();
        assert_eq!(
            resolve_socket(Some("unix:///tmp/x.sock"), &home)
                .unwrap()
                .to_str(),
            Some("/tmp/x.sock")
        );
        assert_eq!(resolve_socket(Some("tcp://1.2.3.4:2375"), &home), None);

        fs::write(
            home.join(".docker/config.json"),
            r#"{"auths": {"x": {"auth": "secret"}}, "currentContext": "orbstack"}"#,
        )
        .unwrap();
        fs::write(
            home.join(".docker/contexts/meta/abc/meta.json"),
            r#"{"Name": "orbstack", "Endpoints": {"docker": {"Host": "unix:///Users/me/.orbstack/run/docker.sock"}}}"#,
        )
        .unwrap();
        assert_eq!(
            resolve_socket(None, &home).unwrap().to_str(),
            Some("/Users/me/.orbstack/run/docker.sock")
        );

        // Default context: the first existing well-known socket.
        fs::write(
            home.join(".docker/config.json"),
            r#"{"currentContext": "default"}"#,
        )
        .unwrap();
        fs::create_dir_all(home.join(".colima/default")).unwrap();
        fs::write(home.join(".colima/default/docker.sock"), "").unwrap();
        let found = resolve_socket(None, &home).unwrap();
        assert!(found.ends_with("docker.sock"));
        let _ = fs::remove_dir_all(&home);
    }
}
