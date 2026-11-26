//! One running copy of the app: the first one listens on a Unix socket in the config
//! folder; a later launch hands it the folders and files it was asked to open, brings it to
//! the front, and exits.
//!
//! Debug builds use their own socket, so `scripts/run.sh` works beside the installed app.

use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui_kit::*;

use crate::inbox::Inbox;

/// A launch request is a few paths; anything longer is not ours.
const MAX_REQUEST: u64 = 64 * 1024;
const TIMEOUT: Duration = Duration::from_secs(2);
/// The first line of every request, so stray connections are ignored.
const HELLO: &str = "power-idea-open v1";

pub enum Launch {
    /// This is the only copy: it keeps the listener and runs the app.
    First(UnixListener),
    /// Another copy took the request; this process should exit.
    Forwarded,
}

fn socket_name() -> &'static str {
    if cfg!(debug_assertions) {
        "instance-dev.sock"
    } else {
        "instance.sock"
    }
}

pub fn socket_path() -> Option<PathBuf> {
    crate::config::Config::dir().map(|dir| dir.join(socket_name()))
}

/// Becomes the running copy, or hands `paths` to the one already running.
/// Any failure falls back to running normally: never refuse to start.
pub fn acquire(socket: &Path, paths: &[PathBuf]) -> Option<Launch> {
    // A copy that does not answer is hung or stale: its socket is taken over below.
    if let Ok(stream) = UnixStream::connect(socket)
        && forward(stream, paths).is_ok()
    {
        return Some(Launch::Forwarded);
    }
    if let Some(dir) = socket.parent() {
        std::fs::create_dir_all(dir).ok()?;
    }
    // Nobody listens: a crashed copy left the file behind.
    std::fs::remove_file(socket).ok();
    if let Ok(listener) = UnixListener::bind(socket) {
        std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600)).ok();
        return Some(Launch::First(listener));
    }
    // Another copy bound it a moment ago: give it our request instead.
    let stream = UnixStream::connect(socket).ok()?;
    forward(stream, paths).ok().map(|()| Launch::Forwarded)
}

/// Sends the request and waits for the running copy's "ok".
fn forward(mut stream: UnixStream, paths: &[PathBuf]) -> std::io::Result<()> {
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    stream.write_all(encode(paths).as_bytes())?;
    stream.shutdown(std::net::Shutdown::Write)?;
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply)?;
    if reply.trim() == "ok" {
        Ok(())
    } else {
        Err(std::io::Error::other("no answer from the running copy"))
    }
}

/// `HELLO`, then one absolute path per line.
fn encode(paths: &[PathBuf]) -> String {
    let mut text = format!("{HELLO}\n");
    for path in paths {
        let path = std::path::absolute(path).unwrap_or_else(|_| path.clone());
        if let Some(line) = path.to_str().filter(|p| !p.contains('\n')) {
            text.push_str(line);
            text.push('\n');
        }
    }
    text
}

/// The paths of a request, or `None` when it is not one of ours.
fn decode(text: &str) -> Option<Vec<PathBuf>> {
    let mut lines = text.lines();
    if lines.next()? != HELLO {
        return None;
    }
    Some(
        lines
            .filter(|line| line.starts_with('/'))
            .map(PathBuf::from)
            .collect(),
    )
}

/// Reads one request from a later launch and answers it.
fn handle(stream: UnixStream) -> Option<Vec<PathBuf>> {
    stream.set_read_timeout(Some(TIMEOUT)).ok()?;
    let mut text = String::new();
    std::io::Read::read_to_string(&mut std::io::Read::take(&stream, MAX_REQUEST), &mut text).ok()?;
    let paths = decode(&text)?;
    (&stream).write_all(b"ok\n").ok()?;
    Some(paths)
}

/// Serves later launches: their paths open here and the app comes to the front.
pub fn listen(listener: UnixListener, cx: &mut App) {
    let requests: Inbox<Vec<PathBuf>> = Inbox::default();
    let queue = requests.clone();
    let spawned = std::thread::Builder::new()
        .name("single-instance".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                if let Some(paths) = handle(stream) {
                    queue.push(paths);
                }
            }
        });
    if let Err(err) = spawned {
        eprintln!("single instance listener: {err}");
        return;
    }
    cx.spawn(async move |cx| {
        while let Some(batches) = requests.next_batch().await {
            cx.update(|cx| {
                for paths in batches {
                    if paths.is_empty() {
                        reveal(cx);
                    } else {
                        crate::open_files::open_paths(paths, cx);
                    }
                }
                cx.activate(true);
            });
        }
    })
    .detach();
}

/// A plain second launch: bring a window forward (or the Welcome screen when none is open).
fn reveal(cx: &mut App) {
    match cx.windows().first() {
        Some(window) => {
            window.update(cx, |_, window, _| window.activate_window()).ok();
        }
        None => crate::welcome::show(cx),
    }
}

/// Removes the socket when the running copy quits.
pub fn release() {
    if let Some(path) = socket_path() {
        std::fs::remove_file(path).ok();
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{Launch, acquire, decode, encode, handle};

    #[test]
    fn requests_round_trip() {
        let paths = vec![PathBuf::from("/Users/me/project"), PathBuf::from("/tmp/a b.rs")];
        assert_eq!(decode(&encode(&paths)), Some(paths));
        assert_eq!(decode(&encode(&[])), Some(Vec::new()));
        assert_eq!(decode("GET / HTTP/1.1\n/etc/passwd\n"), None);
        assert_eq!(
            decode("power-idea-open v1\nrelative\n/abs\n"),
            Some(vec![PathBuf::from("/abs")])
        );
    }

    #[test]
    fn a_second_launch_forwards_to_the_first() {
        let dir = std::env::temp_dir().join(format!("pi-single-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("i.sock");
        let Some(Launch::First(listener)) = acquire(&socket, &[]) else {
            panic!("the first launch should own the socket");
        };
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            handle(stream)
        });
        let wanted = vec![PathBuf::from("/Users/me/notes.md")];
        assert!(matches!(acquire(&socket, &wanted), Some(Launch::Forwarded)));
        assert_eq!(server.join().unwrap(), Some(wanted));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_stale_socket_is_taken_over() {
        let dir = std::env::temp_dir().join(format!("pi-stale-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("i.sock");
        // A crashed copy: the file exists, nobody listens.
        drop(std::os::unix::net::UnixListener::bind(&socket).unwrap());
        assert!(matches!(acquire(&socket, &[]), Some(Launch::First(_))));
        std::fs::remove_dir_all(dir).ok();
    }
}
