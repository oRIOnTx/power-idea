//! Minimal HTTP/1.1 over a unix socket — enough for the Docker Engine API:
//! one request per connection, `Content-Length` or chunked bodies, and
//! streaming bodies (events) read incrementally.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};

const TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

impl Response {
    /// The body as JSON, or an error carrying Docker's `message` for 4xx/5xx.
    pub fn json(self) -> Result<serde_json::Value> {
        let value: serde_json::Value = if self.body.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&self.body).context("decoding Docker's answer")?
        };
        if self.status >= 400 {
            let message = value
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("request failed");
            bail!("{message} (HTTP {})", self.status);
        }
        Ok(value)
    }

    pub fn ok(self) -> Result<()> {
        // 304 = already in that state (e.g. starting a running container).
        if self.status >= 400 {
            self.json()?;
        }
        Ok(())
    }
}

/// Reads a chunked transfer-encoded body as a plain byte stream.
pub struct Chunked<R> {
    inner: R,
    left: usize,
    done: bool,
}

impl<R: BufRead> Chunked<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            left: 0,
            done: false,
        }
    }
}

impl<R: BufRead> Read for Chunked<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.done {
            return Ok(0);
        }
        if self.left == 0 {
            let mut line = String::new();
            // Skip the CRLF that ends the previous chunk.
            while line.trim().is_empty() {
                line.clear();
                if self.inner.read_line(&mut line)? == 0 {
                    self.done = true;
                    return Ok(0);
                }
            }
            let size = line.trim().split(';').next().unwrap_or("0");
            self.left = usize::from_str_radix(size, 16)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            if self.left == 0 {
                self.done = true;
                return Ok(0);
            }
        }
        let max = buf.len().min(self.left);
        let n = self.inner.read(&mut buf[..max])?;
        self.left -= n;
        Ok(n)
    }
}

struct Head {
    status: u16,
    chunked: bool,
    length: Option<usize>,
}

fn send(
    socket: &Path,
    method: &str,
    path: &str,
    body: Option<&[u8]>,
    timeout: Option<Duration>,
) -> Result<BufReader<UnixStream>> {
    let mut stream =
        UnixStream::connect(socket).with_context(|| format!("connecting to {}", socket.display()))?;
    stream.set_read_timeout(timeout)?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    let body = body.unwrap_or_default();
    // One write, so a peer that answers right after the headers never sees a half request.
    let mut message = format!(
        "{method} {path} HTTP/1.1\r\nHost: docker\r\nUser-Agent: power-idea\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    message.extend_from_slice(body);
    stream.write_all(&message)?;
    stream.flush()?;
    Ok(BufReader::new(stream))
}

fn read_head(reader: &mut impl BufRead) -> Result<Head> {
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let status = line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .with_context(|| format!("bad HTTP status line: {line:?}"))?;
    let (mut chunked, mut length) = (false, None);
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 || line.trim().is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let (name, value) = (name.trim().to_ascii_lowercase(), value.trim());
        if name == "transfer-encoding" && value.eq_ignore_ascii_case("chunked") {
            chunked = true;
        } else if name == "content-length" {
            length = value.parse().ok();
        }
    }
    Ok(Head {
        status,
        chunked,
        length,
    })
}

/// Sends a request and reads the whole response.
pub fn request(socket: &Path, method: &str, path: &str, body: Option<&[u8]>) -> Result<Response> {
    let mut reader = send(socket, method, path, body, Some(TIMEOUT))?;
    let head = read_head(&mut reader)?;
    let mut body = Vec::new();
    match (head.chunked, head.length) {
        (true, _) => {
            Chunked::new(reader).read_to_end(&mut body)?;
        }
        (false, Some(length)) => {
            body.resize(length, 0);
            reader.read_exact(&mut body)?;
        }
        (false, None) => {
            reader.read_to_end(&mut body)?;
        }
    }
    Ok(Response {
        status: head.status,
        body,
    })
}

/// Opens a long-running GET (e.g. `/events`) and returns its body as a reader.
pub fn stream(socket: &Path, path: &str) -> Result<Box<dyn BufRead + Send>> {
    let mut reader = send(socket, "GET", path, None, None)?;
    let head = read_head(&mut reader)?;
    if head.status >= 400 {
        bail!("{path} failed (HTTP {})", head.status);
    }
    Ok(if head.chunked {
        Box::new(BufReader::new(Chunked::new(reader)))
    } else {
        Box::new(reader)
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::{BufRead, Read, Write};
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;
    use std::thread;

    use super::{Chunked, request, stream};

    /// A one-shot fake Docker: answers the next connection with `response`
    /// and returns the request it received.
    pub(crate) fn fake_docker(name: &str, response: &'static str) -> (PathBuf, thread::JoinHandle<String>) {
        let path = std::env::temp_dir().join(format!("pi-docker-{name}-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let handle = thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buf = [0u8; 4096];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = conn.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&buf[..n]);
            }
            conn.write_all(response.as_bytes()).unwrap();
            String::from_utf8_lossy(&request).into_owned()
        });
        (path, handle)
    }

    #[test]
    fn decodes_chunked_bodies() {
        let data = "4\r\nWiki\r\n6\r\npedia \r\n0\r\n\r\n";
        let mut out = String::new();
        Chunked::new(data.as_bytes()).read_to_string(&mut out).unwrap();
        assert_eq!(out, "Wikipedia ");
    }

    #[test]
    fn sends_requests_and_reads_responses() {
        let (socket, server) = fake_docker(
            "req",
            "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: 34\r\n\r\n{\"message\":\"No such container: x\"}",
        );
        let response = request(&socket, "POST", "/containers/x/start", None).unwrap();
        assert_eq!(response.status, 404);
        assert_eq!(
            response.json().unwrap_err().to_string(),
            "No such container: x (HTTP 404)"
        );
        assert!(
            server
                .join()
                .unwrap()
                .starts_with("POST /containers/x/start HTTP/1.1\r\n")
        );
    }

    #[test]
    fn streams_chunked_events() {
        let (socket, server) = fake_docker(
            "stream",
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n\
             d\r\n{\"Action\":1}\n\r\nd\r\n{\"Action\":2}\n\r\n0\r\n\r\n",
        );
        let lines: Vec<String> = stream(&socket, "/events")
            .unwrap()
            .lines()
            .map(Result::unwrap)
            .collect();
        assert_eq!(lines, ["{\"Action\":1}", "{\"Action\":2}"]);
        server.join().unwrap();
    }
}
