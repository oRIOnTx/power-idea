//! SFTP over SSH (russh + russh-sftp). Host keys are checked against known_hosts and
//! never accepted silently; SSH keepalives detect dropped connections.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::client::{self, Handle};
use russh::keys::agent::AgentIdentity;
use russh::keys::agent::client::AgentClient;
use russh::keys::{self, HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh_sftp::client::SftpSession;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::{FileAttributes, StatusCode};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use crate::error::{ErrorKind, HostKey, RemoteError, Result, kind_from_message};
use crate::fs::{Entry, EntryKind, Progress, RemoteFs};
use crate::path;
use crate::runtime;

/// A request with no answer for this long counts as a dropped connection.
const REQUEST_TIMEOUT_SECS: u64 = 30;
const KEEPALIVE: Duration = Duration::from_secs(15);
const CHUNK: usize = 64 * 1024;

/// How to log in. `Debug` never prints secrets.
pub enum SshAuth {
    Password(String),
    Key {
        path: PathBuf,
        passphrase: Option<String>,
    },
    Agent,
}

impl std::fmt::Debug for SshAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SshAuth::Password(_) => f.write_str("Password(***)"),
            SshAuth::Key { path, .. } => write!(f, "Key({})", path.display()),
            SshAuth::Agent => f.write_str("Agent"),
        }
    }
}

#[derive(Debug)]
pub struct SshTarget {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: SshAuth,
    pub known_hosts: PathBuf,
    /// A fingerprint the user accepted for this session after seeing a changed key.
    pub trusted_fingerprint: Option<String>,
    pub connect_timeout: Duration,
}

/// Checks the server key; what it saw is kept for the error the UI turns into a dialog.
struct HostKeyCheck {
    host: String,
    port: u16,
    known_hosts: PathBuf,
    trusted: Option<String>,
    rejected: Arc<Mutex<Option<ErrorKind>>>,
}

impl client::Handler for HostKeyCheck {
    type Error = russh::Error;

    fn check_server_key(
        &mut self,
        server_key: &PublicKeyOrCertificate,
    ) -> impl Future<Output = Result<bool, Self::Error>> + Send {
        let accepted = self.verify(server_key);
        async move { Ok(accepted) }
    }
}

impl HostKeyCheck {
    /// Whether to accept the key; a rejection's reason is kept in `rejected`.
    fn verify(&self, server_key: &PublicKeyOrCertificate) -> bool {
        let key = server_key.public_key();
        let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
        if self.trusted.as_deref() == Some(fingerprint.as_str()) {
            return true;
        }
        let host_key = || {
            Box::new(HostKey {
                host: self.host.clone(),
                port: self.port,
                fingerprint: fingerprint.clone(),
                algorithm: key.algorithm().to_string(),
                openssh: key.to_openssh().unwrap_or_default(),
            })
        };
        let verdict = match keys::check_known_hosts_path(&self.host, self.port, &key, &self.known_hosts) {
            Ok(true) => return true,
            Ok(false) => ErrorKind::HostKeyUnknown(host_key()),
            Err(keys::Error::KeyChanged { line }) => ErrorKind::HostKeyChanged {
                key: host_key(),
                line,
            },
            Err(err) => {
                log::warn!("reading {}: {err}", self.known_hosts.display());
                ErrorKind::HostKeyUnknown(host_key())
            }
        };
        if let Ok(mut slot) = self.rejected.lock() {
            *slot = Some(verdict);
        }
        false
    }
}

/// Records an accepted host key in known_hosts, like answering "yes" to `ssh`.
pub fn trust_host_key(key: &HostKey, known_hosts: &std::path::Path) -> Result<()> {
    let public = keys::ssh_key::PublicKey::from_openssh(&key.openssh)
        .map_err(|err| RemoteError::other(format!("invalid host key: {err}")))?;
    keys::known_hosts::learn_known_hosts_path(&key.host, key.port, &public, known_hosts)
        .map_err(|err| RemoteError::other(format!("cannot update {}: {err}", known_hosts.display())))
}

pub struct SftpFs {
    handle: Mutex<Handle<HostKeyCheck>>,
    sftp: SftpSession,
    home: String,
}

impl SftpFs {
    pub fn connect(target: &SshTarget) -> Result<Self> {
        runtime()?.block_on(async {
            tokio::time::timeout(target.connect_timeout, Self::connect_async(target))
                .await
                .map_err(|_| {
                    RemoteError::network(format!("{}:{} did not answer in time", target.host, target.port))
                })?
        })
    }

    async fn connect_async(target: &SshTarget) -> Result<Self> {
        let config = client::Config {
            keepalive_interval: Some(KEEPALIVE),
            keepalive_max: 3,
            inactivity_timeout: None,
            ..Default::default()
        };
        let rejected = Arc::new(Mutex::new(None));
        let check = HostKeyCheck {
            host: target.host.clone(),
            port: target.port,
            known_hosts: target.known_hosts.clone(),
            trusted: target.trusted_fingerprint.clone(),
            rejected: rejected.clone(),
        };
        let mut handle =
            match client::connect(Arc::new(config), (target.host.as_str(), target.port), check).await {
                Ok(handle) => handle,
                Err(err) => {
                    let verdict = rejected.lock().ok().and_then(|mut slot| slot.take());
                    return Err(match verdict {
                        Some(kind) => RemoteError::new(kind, "host key not trusted"),
                        None => ssh_error(&err),
                    });
                }
            };
        authenticate(&mut handle, target).await?;
        let channel = handle.channel_open_session().await.map_err(|e| ssh_error(&e))?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|e| ssh_error(&e))?;
        let sftp = SftpSession::new(channel.into_stream())
            .await
            .map_err(sftp_error)?;
        sftp.set_timeout(REQUEST_TIMEOUT_SECS);
        let home = sftp.canonicalize(".").await.map_err(sftp_error)?;
        Ok(Self {
            handle: Mutex::new(handle),
            sftp,
            home,
        })
    }

    fn block<T>(&self, fut: impl Future<Output = Result<T>>) -> Result<T> {
        runtime()?.block_on(fut)
    }

    async fn entry(&self, path: &str, attrs: FileAttributes) -> Entry {
        let mut kind = if attrs.is_dir() {
            EntryKind::Dir
        } else {
            EntryKind::File
        };
        let symlink = attrs.is_symlink();
        if symlink {
            // What the link points to decides whether it can be expanded; a broken link is a file.
            if let Ok(target) = self.sftp.metadata(path).await
                && target.is_dir()
            {
                kind = EntryKind::Dir;
            }
        }
        Entry {
            name: path::file_name(path).to_string(),
            path: path.to_string(),
            kind,
            symlink,
            size: attrs.size.unwrap_or(0),
            modified: attrs.mtime.map(u64::from),
            permissions: attrs.permissions.map(|p| p & 0o7777),
            owner: attrs.user.clone().or_else(|| attrs.uid.map(|u| u.to_string())),
            group: attrs.group.clone().or_else(|| attrs.gid.map(|g| g.to_string())),
        }
    }
}

async fn authenticate(handle: &mut Handle<HostKeyCheck>, target: &SshTarget) -> Result<()> {
    let user = target.user.clone();
    let rsa_hash = || async { handle_rsa_hash(handle).await };
    let success = match &target.auth {
        SshAuth::Password(password) => handle
            .authenticate_password(user, password.clone())
            .await
            .map_err(|e| ssh_error(&e))?
            .success(),
        SshAuth::Key { path, passphrase } => {
            let key = keys::load_secret_key(path, passphrase.as_deref()).map_err(|err| match err {
                keys::Error::KeyIsEncrypted => {
                    RemoteError::new(ErrorKind::Key, "the key is encrypted: enter its passphrase")
                }
                other => RemoteError::new(ErrorKind::Key, format!("{}: {other}", path.display())),
            })?;
            let hash = rsa_hash().await;
            handle
                .authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
                .await
                .map_err(|e| ssh_error(&e))?
                .success()
        }
        SshAuth::Agent => {
            let mut agent = AgentClient::connect_env().await.map_err(|err| {
                RemoteError::new(
                    ErrorKind::Key,
                    format!("ssh-agent is not available (SSH_AUTH_SOCK): {err}"),
                )
            })?;
            let identities = agent
                .request_identities()
                .await
                .map_err(|err| RemoteError::new(ErrorKind::Key, format!("ssh-agent: {err}")))?;
            if identities.is_empty() {
                return Err(RemoteError::new(
                    ErrorKind::Key,
                    "ssh-agent has no keys (add one with ssh-add)",
                ));
            }
            let hash = rsa_hash().await;
            let mut ok = false;
            for identity in identities {
                let AgentIdentity::PublicKey { key, .. } = identity else {
                    continue;
                };
                let result = handle
                    .authenticate_publickey_with(user.clone(), key, hash, &mut agent)
                    .await;
                if result.is_ok_and(|r| r.success()) {
                    ok = true;
                    break;
                }
            }
            ok
        }
    };
    if success {
        Ok(())
    } else {
        Err(RemoteError::new(
            ErrorKind::Auth,
            format!("login as {} failed", target.user),
        ))
    }
}

async fn handle_rsa_hash(handle: &Handle<HostKeyCheck>) -> Option<HashAlg> {
    handle.best_supported_rsa_hash().await.ok().flatten().flatten()
}

fn ssh_error(err: &russh::Error) -> RemoteError {
    match err {
        russh::Error::IO(io) => RemoteError::from(std::io::Error::new(io.kind(), io.to_string())),
        russh::Error::Disconnect | russh::Error::SendError | russh::Error::ConnectionTimeout => {
            RemoteError::network(err.to_string())
        }
        other => RemoteError::other(other.to_string()),
    }
}

fn sftp_error(err: SftpError) -> RemoteError {
    match err {
        SftpError::Status(status) => {
            let kind = match status.status_code {
                StatusCode::NoSuchFile => ErrorKind::NotFound,
                StatusCode::PermissionDenied => ErrorKind::PermissionDenied,
                StatusCode::NoConnection | StatusCode::ConnectionLost => ErrorKind::Network,
                _ => kind_from_message(&status.error_message),
            };
            RemoteError::new(kind, status.error_message)
        }
        SftpError::Timeout => RemoteError::network("the server stopped answering"),
        SftpError::IO(message) => RemoteError::network(message),
        other => RemoteError::other(other.to_string()),
    }
}

impl RemoteFs for SftpFs {
    fn home(&self) -> Result<String> {
        Ok(self.home.clone())
    }

    fn list(&self, dir: &str) -> Result<Vec<Entry>> {
        self.block(async {
            let read = self.sftp.read_dir(dir).await.map_err(sftp_error)?;
            let mut entries = Vec::new();
            for item in read {
                let name = item.file_name();
                if name == "." || name == ".." {
                    continue;
                }
                entries.push(self.entry(&path::join(dir, &name), item.metadata()).await);
            }
            Ok(entries)
        })
    }

    fn stat(&self, path: &str) -> Result<Entry> {
        self.block(async {
            let attrs = self.sftp.symlink_metadata(path).await.map_err(sftp_error)?;
            Ok(self.entry(path, attrs).await)
        })
    }

    fn download(&self, path: &str, out: &mut dyn Write, progress: Progress<'_>) -> Result<u64> {
        self.block(async {
            let mut file = self.sftp.open(path).await.map_err(sftp_error)?;
            let mut buf = vec![0u8; CHUNK];
            let mut total = 0u64;
            loop {
                let n = file.read(&mut buf).await.map_err(RemoteError::from)?;
                if n == 0 {
                    break;
                }
                out.write_all(buf.get(..n).unwrap_or_default())?;
                total += n as u64;
                if !progress(total) {
                    return Err(RemoteError::cancelled());
                }
            }
            file.shutdown().await.ok();
            out.flush()?;
            Ok(total)
        })
    }

    fn upload(&self, path: &str, input: &mut dyn Read, progress: Progress<'_>) -> Result<u64> {
        self.block(async {
            // CREATE | TRUNCATE | WRITE: an existing file keeps its permissions and owner.
            let mut file = self.sftp.create(path).await.map_err(sftp_error)?;
            let mut buf = vec![0u8; CHUNK];
            let mut total = 0u64;
            loop {
                let n = input.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                file.write_all(buf.get(..n).unwrap_or_default())
                    .await
                    .map_err(RemoteError::from)?;
                total += n as u64;
                if !progress(total) {
                    file.shutdown().await.ok();
                    return Err(RemoteError::cancelled());
                }
            }
            file.shutdown().await.map_err(RemoteError::from)?;
            Ok(total)
        })
    }

    fn rename(&self, from: &str, to: &str) -> Result<()> {
        self.block(async { self.sftp.rename(from, to).await.map_err(sftp_error) })
    }

    fn remove_file(&self, path: &str) -> Result<()> {
        self.block(async { self.sftp.remove_file(path).await.map_err(sftp_error) })
    }

    fn remove_dir(&self, path: &str) -> Result<()> {
        self.block(async { self.sftp.remove_dir(path).await.map_err(sftp_error) })
    }

    fn mkdir(&self, path: &str) -> Result<()> {
        self.block(async { self.sftp.create_dir(path).await.map_err(sftp_error) })
    }

    fn chmod(&self, path: &str, mode: u32) -> Result<()> {
        let attrs = FileAttributes {
            permissions: Some(mode & 0o7777),
            ..FileAttributes::empty()
        };
        self.block(async { self.sftp.set_metadata(path, attrs).await.map_err(sftp_error) })
    }

    fn is_alive(&self) -> bool {
        self.handle.lock().is_ok_and(|handle| !handle.is_closed())
    }
}
