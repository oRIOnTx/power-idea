//! Remote errors, sorted into kinds the UI can explain in plain language and act on
//! (ask about a host key, offer a reconnect, show a conflict).

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorKind {
    /// Host not found, refused, timed out, or the connection dropped.
    Network,
    /// The server rejected the user name, password, key or agent identities.
    Auth,
    /// The host key is not in known_hosts: the user must confirm the fingerprint.
    HostKeyUnknown(Box<HostKey>),
    /// The host key differs from the one recorded in known_hosts (at `line`).
    HostKeyChanged {
        key: Box<HostKey>,
        line: usize,
    },
    NotFound,
    PermissionDenied,
    /// The operation was cancelled by the user.
    Cancelled,
    /// A private key or passphrase problem on this machine.
    Key,
    Other,
}

/// A server's public host key, as shown to the user before trusting it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostKey {
    pub host: String,
    pub port: u16,
    /// `SHA256:…`, as `ssh` prints it.
    pub fingerprint: String,
    pub algorithm: String,
    /// The key in OpenSSH format (public, not a secret), to record in known_hosts.
    pub openssh: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteError {
    pub kind: ErrorKind,
    /// Technical detail for the error journal; never contains secrets.
    pub detail: String,
}

pub type Result<T, E = RemoteError> = std::result::Result<T, E>;

impl RemoteError {
    pub fn new(kind: ErrorKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
        }
    }

    pub fn other(detail: impl Into<String>) -> Self {
        Self::new(ErrorKind::Other, detail)
    }

    pub fn network(detail: impl Into<String>) -> Self {
        Self::new(ErrorKind::Network, detail)
    }

    pub fn cancelled() -> Self {
        Self::new(ErrorKind::Cancelled, "cancelled")
    }

    /// Whether reconnecting and retrying could help.
    pub fn is_connection_lost(&self) -> bool {
        self.kind == ErrorKind::Network
    }

    /// One sentence for a notification.
    pub fn user_message(&self) -> String {
        match &self.kind {
            ErrorKind::Network => format!("Cannot reach the server: {}", self.detail),
            ErrorKind::Auth => "The server rejected the login. Check the user name and credentials.".into(),
            ErrorKind::HostKeyUnknown(key) => {
                format!("The server's host key is not known yet ({})", key.fingerprint)
            }
            ErrorKind::HostKeyChanged { key, line } => format!(
                "The server's host key changed (now {}; known_hosts line {line}). \
                 This can mean someone is intercepting the connection.",
                key.fingerprint
            ),
            ErrorKind::NotFound => format!("Not found: {}", self.detail),
            ErrorKind::PermissionDenied => format!("Permission denied: {}", self.detail),
            ErrorKind::Cancelled => "Cancelled".into(),
            ErrorKind::Key => format!("Cannot use the private key: {}", self.detail),
            ErrorKind::Other => self.detail.clone(),
        }
    }
}

impl fmt::Display for RemoteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.user_message())
    }
}

impl std::error::Error for RemoteError {}

impl From<std::io::Error> for RemoteError {
    fn from(err: std::io::Error) -> Self {
        use std::io::ErrorKind as Io;
        let kind = match err.kind() {
            Io::NotFound => ErrorKind::NotFound,
            Io::PermissionDenied => ErrorKind::PermissionDenied,
            Io::ConnectionRefused
            | Io::ConnectionReset
            | Io::ConnectionAborted
            | Io::NotConnected
            | Io::BrokenPipe
            | Io::TimedOut
            | Io::UnexpectedEof
            | Io::HostUnreachable
            | Io::NetworkUnreachable
            | Io::NetworkDown
            | Io::AddrNotAvailable => ErrorKind::Network,
            _ => ErrorKind::Other,
        };
        Self::new(kind, err.to_string())
    }
}

/// Sorts a server's status text (SFTP status message, FTP reply) into a kind.
pub(crate) fn kind_from_message(message: &str) -> ErrorKind {
    let lower = message.to_ascii_lowercase();
    if lower.contains("no such file") || lower.contains("not found") || lower.contains("does not exist") {
        ErrorKind::NotFound
    } else if lower.contains("permission denied") || lower.contains("access denied") {
        ErrorKind::PermissionDenied
    } else {
        ErrorKind::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_errors_map_to_kinds() {
        let refused = std::io::Error::from(std::io::ErrorKind::ConnectionRefused);
        assert!(RemoteError::from(refused).is_connection_lost());
        let missing = std::io::Error::from(std::io::ErrorKind::NotFound);
        assert_eq!(RemoteError::from(missing).kind, ErrorKind::NotFound);
    }

    #[test]
    fn server_messages_map_to_kinds() {
        assert_eq!(kind_from_message("No such file"), ErrorKind::NotFound);
        assert_eq!(
            kind_from_message("Permission denied"),
            ErrorKind::PermissionDenied
        );
        assert_eq!(kind_from_message("Failure"), ErrorKind::Other);
    }

    #[test]
    fn messages_are_plain_language() {
        let err = RemoteError::new(ErrorKind::Auth, "publickey");
        assert!(err.user_message().contains("rejected the login"));
        let key = HostKey {
            host: "h".into(),
            port: 22,
            fingerprint: "SHA256:abc".into(),
            algorithm: "ssh-ed25519".into(),
            openssh: String::new(),
        };
        let changed = RemoteError::new(
            ErrorKind::HostKeyChanged {
                key: Box::new(key),
                line: 3,
            },
            "",
        );
        assert!(changed.user_message().contains("line 3"));
    }
}
