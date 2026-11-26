//! FTP and FTPS (explicit TLS) through suppaftp's blocking client. One control
//! connection per server, so operations take turns behind a mutex.

use std::io::{Read, Write};
use std::net::ToSocketAddrs as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, UNIX_EPOCH};

use suppaftp::list::{File as ListFile, ListParser, PosixPexQuery};
use suppaftp::types::FileType;
use suppaftp::{FtpError, FtpStream, ImplFtpStream, RustlsConnector, RustlsFtpStream, TlsStream};

use crate::error::{ErrorKind, RemoteError, Result, kind_from_message};
use crate::fs::{Entry, EntryKind, Progress, RemoteFs, copy_with_progress};
use crate::path;

pub struct FtpTarget {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub tls: bool,
    pub connect_timeout: Duration,
}

impl std::fmt::Debug for FtpTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "FtpTarget({}@{}:{}, tls: {})",
            self.user, self.host, self.port, self.tls
        )
    }
}

enum Conn {
    Plain(FtpStream),
    Tls(RustlsFtpStream),
}

/// Runs `$body` with `$s` bound to whichever stream type the connection uses.
macro_rules! with_stream {
    ($self:ident, $s:ident => $body:expr) => {{
        let mut guard = $self.lock()?;
        let result = match &mut *guard {
            Conn::Plain($s) => $body,
            Conn::Tls($s) => $body,
        };
        $self.track(result)
    }};
}

pub struct FtpFs {
    conn: Mutex<Conn>,
    home: String,
    mlsd: bool,
    alive: AtomicBool,
}

impl FtpFs {
    pub fn connect(target: &FtpTarget) -> Result<Self> {
        let addr = (target.host.as_str(), target.port)
            .to_socket_addrs()
            .map_err(|err| RemoteError::network(format!("{}: {err}", target.host)))?
            .next()
            .ok_or_else(|| RemoteError::network(format!("{} has no address", target.host)))?;
        let mut conn = if target.tls {
            let stream = RustlsFtpStream::connect_timeout(addr, target.connect_timeout)
                .map_err(ftp_error)?
                .into_secure(RustlsConnector::from(tls_config()?), &target.host)
                .map_err(ftp_error)?;
            Conn::Tls(stream)
        } else {
            Conn::Plain(FtpStream::connect_timeout(addr, target.connect_timeout).map_err(ftp_error)?)
        };
        let (home, mlsd) = match &mut conn {
            Conn::Plain(s) => setup(s, target)?,
            Conn::Tls(s) => setup(s, target)?,
        };
        Ok(Self {
            conn: Mutex::new(conn),
            home,
            mlsd,
            alive: AtomicBool::new(true),
        })
    }

    fn lock(&self) -> Result<MutexGuard<'_, Conn>> {
        self.conn
            .lock()
            .map_err(|_| RemoteError::other("FTP connection state is broken; reconnect"))
    }

    /// Marks the connection dead on network errors, so the next call reconnects.
    fn track<T>(&self, result: Result<T>) -> Result<T> {
        if let Err(err) = &result
            && err.is_connection_lost()
        {
            self.alive.store(false, Ordering::Relaxed);
        }
        result
    }

    fn parse_listing(&self, dir: &str, lines: &[String]) -> Vec<Entry> {
        lines
            .iter()
            .filter_map(|line| {
                if self.mlsd {
                    ListParser::parse_mlsd(line).ok()
                } else {
                    ListParser::parse_posix(line)
                        .or_else(|_| ListParser::parse_dos(line))
                        .ok()
                }
            })
            .filter(|f| f.name() != "." && f.name() != ".." && !f.name().contains('/'))
            .map(|f| entry(dir, &f))
            .collect()
    }
}

fn setup<T: TlsStream>(stream: &mut ImplFtpStream<T>, target: &FtpTarget) -> Result<(String, bool)> {
    stream
        .login(target.user.as_str(), target.password.as_str())
        .map_err(|err| match ftp_error(err) {
            e if e.kind == ErrorKind::Other => RemoteError::new(ErrorKind::Auth, e.detail),
            e => e,
        })?;
    stream.transfer_type(FileType::Binary).map_err(ftp_error)?;
    let home = stream.pwd().map_err(ftp_error)?;
    let mlsd = stream
        .feat()
        .is_ok_and(|features| features.keys().any(|k| k.eq_ignore_ascii_case("MLST")));
    Ok((path::normalize("/", &home), mlsd))
}

/// Root certificates from the macOS Keychain (trusted by the system).
fn tls_config() -> Result<Arc<rustls::ClientConfig>> {
    let mut roots = rustls::RootCertStore::empty();
    let found = rustls_native_certs::load_native_certs();
    for cert in found.certs {
        roots.add(cert).ok();
    }
    if roots.is_empty() {
        return Err(RemoteError::other("no trusted root certificates found for FTPS"));
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|err| RemoteError::other(format!("TLS setup: {err}")))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Arc::new(config))
}

fn entry(dir: &str, file: &ListFile) -> Entry {
    let mode = [PosixPexQuery::Owner, PosixPexQuery::Group, PosixPexQuery::Others]
        .iter()
        .fold(0u32, |mode, who| {
            let bits = u32::from(file.can_read(*who)) << 2
                | u32::from(file.can_write(*who)) << 1
                | u32::from(file.can_execute(*who));
            (mode << 3) | bits
        });
    Entry {
        name: file.name().to_string(),
        path: path::join(dir, file.name()),
        // A symlink's target type is unknown over FTP; links usually point at folders on web hosts.
        kind: if file.is_directory() || file.is_symlink() {
            EntryKind::Dir
        } else {
            EntryKind::File
        },
        symlink: file.is_symlink(),
        size: file.size() as u64,
        modified: file
            .modified()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|d| d.as_secs()),
        permissions: (mode != 0).then_some(mode),
        owner: file.uid().map(|u| u.to_string()),
        group: file.gid().map(|g| g.to_string()),
    }
}

fn ftp_error(err: FtpError) -> RemoteError {
    match err {
        FtpError::ConnectionError(io) => RemoteError::from(io),
        FtpError::UnexpectedResponse(response) => {
            let text = String::from_utf8_lossy(&response.body).trim().to_string();
            let code = response.status.code();
            let kind = match code {
                530 => ErrorKind::Auth,
                421 | 425 | 426 => ErrorKind::Network,
                _ => match kind_from_message(&text) {
                    ErrorKind::Other if code == 550 => ErrorKind::NotFound,
                    kind => kind,
                },
            };
            RemoteError::new(kind, text)
        }
        FtpError::SecureError(text) => RemoteError::other(format!("TLS: {text}")),
        other => RemoteError::other(other.to_string()),
    }
}

impl RemoteFs for FtpFs {
    fn home(&self) -> Result<String> {
        Ok(self.home.clone())
    }

    fn list(&self, dir: &str) -> Result<Vec<Entry>> {
        let lines = if self.mlsd {
            with_stream!(self, s => s.mlsd(Some(dir)).map_err(ftp_error))?
        } else {
            with_stream!(self, s => s.list(Some(dir)).map_err(ftp_error))?
        };
        Ok(self.parse_listing(dir, &lines))
    }

    fn stat(&self, target: &str) -> Result<Entry> {
        let target = path::normalize("/", target);
        if target == "/" {
            return Ok(Entry {
                name: String::new(),
                path: "/".into(),
                kind: EntryKind::Dir,
                symlink: false,
                size: 0,
                modified: None,
                permissions: None,
                owner: None,
                group: None,
            });
        }
        // Listing the parent works on every server (MLST and SIZE/MDTM are optional).
        let name = path::file_name(&target);
        self.list(&path::parent(&target))?
            .into_iter()
            .find(|e| e.name == name)
            .ok_or_else(|| RemoteError::new(ErrorKind::NotFound, target.clone()))
    }

    fn download(&self, remote: &str, out: &mut dyn Write, progress: Progress<'_>) -> Result<u64> {
        with_stream!(self, s => {
            match s.retr_as_stream(remote) {
                Ok(mut stream) => {
                    let copied = copy_with_progress(&mut stream, out, progress);
                    let finished = stream.finish().map_err(ftp_error);
                    copied.and_then(|n| finished.map(|()| n))
                }
                Err(err) => Err(ftp_error(err)),
            }
        })
    }

    fn upload(&self, remote: &str, input: &mut dyn Read, progress: Progress<'_>) -> Result<u64> {
        with_stream!(self, s => {
            match s.put_with_stream(remote) {
                Ok(mut stream) => {
                    let copied = copy_with_progress(input, &mut stream, progress);
                    let finished = stream.finish().map_err(ftp_error);
                    copied.and_then(|n| finished.map(|()| n))
                }
                Err(err) => Err(ftp_error(err)),
            }
        })
    }

    fn rename(&self, from: &str, to: &str) -> Result<()> {
        with_stream!(self, s => s.rename(from, to).map_err(ftp_error))
    }

    fn remove_file(&self, target: &str) -> Result<()> {
        with_stream!(self, s => s.rm(target).map_err(ftp_error))
    }

    fn remove_dir(&self, target: &str) -> Result<()> {
        with_stream!(self, s => s.rmdir(target).map_err(ftp_error))
    }

    fn mkdir(&self, target: &str) -> Result<()> {
        with_stream!(self, s => s.mkdir(target).map_err(ftp_error))
    }

    fn chmod(&self, target: &str, mode: u32) -> Result<()> {
        let command = format!("CHMOD {:o} {target}", mode & 0o7777);
        with_stream!(self, s => s.site(command.as_str()).map(|_| ()).map_err(ftp_error))
    }

    fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }
}
