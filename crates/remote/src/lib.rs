//! Remote Files: browse and edit files on servers over SFTP, FTP and FTPS behind one
//! protocol-neutral [`RemoteFs`] trait. No UI; every call blocks and belongs on a
//! background thread.

pub mod cache;
pub mod conflict;
mod connect;
pub mod error;
mod fs;
mod ftp;
pub mod listing;
pub mod path;
pub mod secrets;
mod session;
mod sftp;
pub mod sites;
pub mod ssh_config;
pub mod transfer;

use std::sync::OnceLock;

pub use connect::{ConnectOptions, Credentials, connect, default_known_hosts};
pub use error::{ErrorKind, HostKey, RemoteError, Result};
pub use fs::{Entry, EntryKind, Progress, RemoteFs};
pub use session::{Connection, Connector, MAX_RECONNECTS};
pub use sftp::trust_host_key;
pub use sites::{Auth, Protocol, Site};

/// The tokio runtime SSH sessions live on (keepalives run on it between calls).
fn runtime() -> Result<&'static tokio::runtime::Runtime> {
    static RUNTIME: OnceLock<std::result::Result<tokio::runtime::Runtime, String>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("remote-ssh")
                .enable_all()
                .build()
                .map_err(|err| err.to_string())
        })
        .as_ref()
        .map_err(|err| RemoteError::other(format!("cannot start the network runtime: {err}")))
}

/// Opens a self-healing connection to `site` (credentials from the Keychain unless given).
pub fn open(site: &Site, credentials: Credentials, options: ConnectOptions) -> Result<Connection> {
    let site = site.clone();
    Connection::open(Box::new(move || connect(&site, &credentials, &options)))
}
