//! Opening a [`RemoteFs`] for a saved site: credentials from the Keychain, the protocol
//! from the site, host keys from `~/.ssh/known_hosts`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::error::{ErrorKind, RemoteError, Result};
use crate::fs::RemoteFs;
use crate::ftp::{FtpFs, FtpTarget};
use crate::secrets::{self, Secret};
use crate::sftp::{SftpFs, SshAuth, SshTarget};
use crate::sites::{Auth, Protocol, Site};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

/// Per-connection choices that are not part of the saved site.
#[derive(Debug, Clone, Default)]
pub struct ConnectOptions {
    /// known_hosts to check against; `None` = `~/.ssh/known_hosts`.
    pub known_hosts: Option<PathBuf>,
    /// A changed host key the user chose to trust for this session only.
    pub trusted_fingerprint: Option<String>,
}

impl ConnectOptions {
    pub fn known_hosts_path(&self) -> PathBuf {
        self.known_hosts.clone().unwrap_or_else(default_known_hosts)
    }
}

pub fn default_known_hosts() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".ssh")
        .join("known_hosts")
}

/// Credentials typed for a connection, overriding the Keychain. `Debug` hides them.
#[derive(Clone, Default)]
pub struct Credentials {
    pub password: Option<String>,
    pub passphrase: Option<String>,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Credentials(***)")
    }
}

impl Credentials {
    /// The site's stored secrets.
    pub fn from_keychain(site: &Site) -> Self {
        Self {
            password: secrets::load(&site.id, Secret::Password),
            passphrase: secrets::load(&site.id, Secret::Passphrase),
        }
    }
}

/// Connects to `site`. Blocks: call it on a background thread.
pub fn connect(
    site: &Site,
    credentials: &Credentials,
    options: &ConnectOptions,
) -> Result<Arc<dyn RemoteFs>> {
    log::info!(
        "remote: connecting to {} ({})",
        site.address(),
        site.protocol.label()
    );
    match site.protocol {
        Protocol::Sftp => {
            let auth = match &site.auth {
                Auth::Password => SshAuth::Password(require_password(credentials)?),
                Auth::Key { path } => SshAuth::Key {
                    path: path.clone(),
                    passphrase: credentials.passphrase.clone(),
                },
                Auth::Agent => SshAuth::Agent,
            };
            let target = SshTarget {
                host: site.host.clone(),
                port: site.port,
                user: site.user.clone(),
                auth,
                known_hosts: options.known_hosts_path(),
                trusted_fingerprint: options.trusted_fingerprint.clone(),
                connect_timeout: CONNECT_TIMEOUT,
            };
            Ok(Arc::new(SftpFs::connect(&target)?))
        }
        Protocol::Ftp | Protocol::Ftps => {
            let user = if site.user.is_empty() {
                "anonymous"
            } else {
                site.user.as_str()
            };
            let password = match credentials.password.clone() {
                Some(password) => password,
                None if user == "anonymous" => String::new(),
                None => require_password(credentials)?,
            };
            let target = FtpTarget {
                host: site.host.clone(),
                port: site.port,
                user: user.to_string(),
                password,
                tls: site.protocol == Protocol::Ftps,
                connect_timeout: CONNECT_TIMEOUT,
            };
            Ok(Arc::new(FtpFs::connect(&target)?))
        }
    }
}

fn require_password(credentials: &Credentials) -> Result<String> {
    credentials
        .password
        .clone()
        .ok_or_else(|| RemoteError::new(ErrorKind::Auth, "no password saved for this site"))
}
