//! Saved connections ("sites") in `remote-sites.toml` next to the user config.
//! Only non-secret settings live here; passwords and passphrases go to the Keychain.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    #[default]
    Sftp,
    Ftp,
    /// FTP with explicit TLS (`AUTH TLS`).
    Ftps,
}

impl Protocol {
    pub const ALL: [Protocol; 3] = [Protocol::Sftp, Protocol::Ftp, Protocol::Ftps];

    pub fn label(self) -> &'static str {
        match self {
            Protocol::Sftp => "SFTP",
            Protocol::Ftp => "FTP",
            Protocol::Ftps => "FTPS",
        }
    }

    pub fn default_port(self) -> u16 {
        match self {
            Protocol::Sftp => 22,
            Protocol::Ftp | Protocol::Ftps => 21,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "method", rename_all = "kebab-case")]
pub enum Auth {
    #[default]
    Password,
    /// A private key file; its passphrase (if any) is in the Keychain.
    Key { path: PathBuf },
    /// Identities from the running ssh-agent (`SSH_AUTH_SOCK`).
    Agent,
}

impl Auth {
    pub fn label(&self) -> &'static str {
        match self {
            Auth::Password => "Password",
            Auth::Key { .. } => "Private key",
            Auth::Agent => "ssh-agent",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Site {
    /// Stable id: Keychain account and cache folder name.
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub protocol: Protocol,
    pub host: String,
    pub port: u16,
    pub user: String,
    #[serde(default)]
    pub auth: Auth,
    /// Folder shown after connecting; empty = the server's home folder.
    #[serde(default)]
    pub remote_path: String,
    /// A local folder this server mirrors (e.g. the project), for uploads and downloads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_path: Option<PathBuf>,
    /// A label color name from the theme palette (`red`, `green`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Lets AI agents list and read this server's files through the IDE's MCP server.
    #[serde(default)]
    pub ai_read_access: bool,
}

impl Site {
    pub fn new(name: &str, protocol: Protocol, host: &str, user: &str) -> Self {
        Self {
            id: new_id(name),
            name: name.into(),
            protocol,
            host: host.into(),
            port: protocol.default_port(),
            user: user.into(),
            auth: if protocol == Protocol::Sftp {
                Auth::Agent
            } else {
                Auth::Password
            },
            remote_path: String::new(),
            local_path: None,
            color: None,
            ai_read_access: false,
        }
    }

    /// `user@host` (with `:port` when not the default), for labels.
    pub fn address(&self) -> String {
        let port = if self.port == self.protocol.default_port() {
            String::new()
        } else {
            format!(":{}", self.port)
        };
        if self.user.is_empty() {
            format!("{}{port}", self.host)
        } else {
            format!("{}@{}{port}", self.user, self.host)
        }
    }

    /// A short problem description, or `None` when the site can be saved.
    pub fn validate(&self) -> Option<&'static str> {
        if self.name.trim().is_empty() {
            return Some("Name is empty");
        }
        if self.host.trim().is_empty() || self.host.contains(char::is_whitespace) {
            return Some("Host is empty or contains spaces");
        }
        if self.port == 0 {
            return Some("Port must be 1–65535");
        }
        if self.user.trim().is_empty() && self.protocol == Protocol::Sftp {
            return Some("User is empty");
        }
        if matches!(&self.auth, Auth::Key { path } if path.as_os_str().is_empty()) {
            return Some("Choose a private key file");
        }
        if self.protocol != Protocol::Sftp && self.auth != Auth::Password {
            return Some("FTP supports password login only");
        }
        if !self.remote_path.is_empty() && !self.remote_path.starts_with('/') {
            return Some("Remote path must start with /");
        }
        None
    }
}

/// An id from the name plus the current time: readable in the Keychain, unique enough.
fn new_id(name: &str) -> String {
    let slug: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .chars()
        .take(24)
        .collect();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!(
        "{}-{:x}",
        if slug.is_empty() { "site" } else { &slug },
        nanos & 0xffff_ffff
    )
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct SitesFile {
    #[serde(default, rename = "site")]
    sites: Vec<Site>,
}

pub fn load(path: &Path) -> Result<Vec<Site>> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err).with_context(|| format!("reading {}", path.display())),
    };
    let file: SitesFile = toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    Ok(file.sites)
}

/// Writes atomically (temp file + rename), so a crash never leaves half a file.
pub fn save(path: &Path, sites: &[Site]) -> Result<()> {
    let ids: std::collections::HashSet<&str> = sites.iter().map(|s| s.id.as_str()).collect();
    if ids.len() != sites.len() {
        bail!("two sites share an id");
    }
    let text = toml::to_string_pretty(&SitesFile {
        sites: sites.to_vec(),
    })?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_toml() {
        let dir = std::env::temp_dir().join(format!("power-idea-sites-{}", std::process::id()));
        let path = dir.join("remote-sites.toml");
        let mut sftp = Site::new("Hetzner box", Protocol::Sftp, "1.2.3.4", "deploy");
        sftp.auth = Auth::Key {
            path: "/Users/me/.ssh/id_ed25519".into(),
        };
        sftp.remote_path = "/srv/app".into();
        let ftp = Site::new("Old FTP", Protocol::Ftps, "ftp.example.com", "web");
        save(&path, &[sftp.clone(), ftp.clone()]).unwrap();
        assert_eq!(load(&path).unwrap(), vec![sftp, ftp]);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("password ="), "no secrets in the file");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn missing_file_is_empty() {
        assert!(
            load(Path::new("/nonexistent/remote-sites.toml"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn validates() {
        let mut site = Site::new("x", Protocol::Sftp, "host", "me");
        assert_eq!(site.validate(), None);
        site.host = "bad host".into();
        assert!(site.validate().is_some());
        site.host = "host".into();
        site.protocol = Protocol::Ftp;
        site.auth = Auth::Agent;
        assert_eq!(site.validate(), Some("FTP supports password login only"));
        site.auth = Auth::Password;
        site.remote_path = "relative".into();
        assert!(site.validate().is_some());
    }

    #[test]
    fn addresses() {
        let mut site = Site::new("x", Protocol::Sftp, "host", "me");
        assert_eq!(site.address(), "me@host");
        site.port = 2222;
        assert_eq!(site.address(), "me@host:2222");
        assert!(site.id.starts_with("x-"));
    }
}
