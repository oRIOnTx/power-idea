//! Hosts from `~/.ssh/config`, to import as sites. Only concrete `Host` aliases are
//! offered (no wildcards); `Match` blocks and `Include` are skipped.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshHost {
    pub alias: String,
    pub hostname: String,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub identity_file: Option<PathBuf>,
}

/// Parses the text of an ssh config; `home` expands `~` in `IdentityFile`.
pub fn parse(text: &str, home: &Path) -> Vec<SshHost> {
    let mut hosts: Vec<SshHost> = Vec::new();
    // Indices into `hosts` the current `Host` line declared (one line can name several).
    let mut current: Vec<usize> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = split_option(line);
        let key = key.to_ascii_lowercase();
        match key.as_str() {
            "host" => {
                current.clear();
                for alias in value.split_whitespace() {
                    if alias.contains(['*', '?', '!']) {
                        continue;
                    }
                    current.push(hosts.len());
                    hosts.push(SshHost {
                        alias: alias.to_string(),
                        hostname: alias.to_string(),
                        user: None,
                        port: None,
                        identity_file: None,
                    });
                }
            }
            "match" => current.clear(),
            _ => {
                for &ix in &current {
                    let Some(host) = hosts.get_mut(ix) else { continue };
                    apply(host, &key, value, home);
                }
            }
        }
    }
    hosts
}

/// Reads and parses the user's `~/.ssh/config` (empty when missing).
pub fn load_user_config() -> Vec<SshHost> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    std::fs::read_to_string(home.join(".ssh").join("config"))
        .map_or_else(|_| Vec::new(), |text| parse(&text, &home))
}

/// `Key value`, `Key=value` or `Key = "value"`.
fn split_option(line: &str) -> (&str, &str) {
    let split = line
        .find(|c: char| c.is_whitespace() || c == '=')
        .unwrap_or(line.len());
    let (key, rest) = line.split_at(split);
    let value = rest
        .trim_start_matches(|c: char| c.is_whitespace() || c == '=')
        .trim();
    (key, value.trim_matches('"'))
}

/// The first value wins, as in OpenSSH.
fn apply(host: &mut SshHost, key: &str, value: &str, home: &Path) {
    match key {
        "hostname" if host.hostname == host.alias => host.hostname = value.to_string(),
        "user" if host.user.is_none() => host.user = Some(value.to_string()),
        "port" if host.port.is_none() => host.port = value.parse().ok(),
        "identityfile" if host.identity_file.is_none() => {
            let path = match value.strip_prefix("~/") {
                Some(rest) => home.join(rest),
                None => PathBuf::from(value),
            };
            host.identity_file = Some(path);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"
# comment
Host *
  ServerAliveInterval 30

Host hetzner prod
  HostName 95.216.1.2
  User deploy
  Port 2222
  IdentityFile ~/.ssh/id_ed25519
  User ignored

Host=staging
  Hostname = "staging.example.com"

Match host foo
  User nope
"#;

    #[test]
    fn parses_concrete_hosts() {
        let hosts = parse(CONFIG, Path::new("/Users/me"));
        assert_eq!(hosts.len(), 3);
        let hetzner = hosts.first().unwrap();
        assert_eq!(hetzner.alias, "hetzner");
        assert_eq!(hetzner.hostname, "95.216.1.2");
        assert_eq!(hetzner.user.as_deref(), Some("deploy"));
        assert_eq!(hetzner.port, Some(2222));
        assert_eq!(
            hetzner.identity_file.as_deref(),
            Some(Path::new("/Users/me/.ssh/id_ed25519"))
        );
        assert_eq!(hosts.get(1).map(|h| h.hostname.as_str()), Some("95.216.1.2"));
        let staging = hosts.get(2).unwrap();
        assert_eq!(staging.hostname, "staging.example.com");
        assert_eq!(staging.user, None);
    }
}
