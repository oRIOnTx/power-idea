//! One HTTP exchange behind a trait, so the GitHub client is tested without the network.
//! Redirects are never followed here: the caller decides which headers go to the new host.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context as _, Result};

#[derive(Clone, PartialEq, Eq)]
pub struct Request {
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
}

impl std::fmt::Debug for Request {
    // Headers carry the token: never print them.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Request")
            .field("method", &self.method)
            .field("url", &self.url)
            .finish_non_exhaustive()
    }
}

impl Request {
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            method: "GET",
            url: url.into(),
            headers: Vec::new(),
            body: None,
        }
    }

    /// A POST with a JSON body.
    pub fn post_json(url: impl Into<String>, body: &serde_json::Value) -> Self {
        Self {
            method: "POST",
            url: url.into(),
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: Some(body.to_string().into_bytes()),
        }
    }

    pub fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push((name.to_string(), value.into()));
        self
    }

    pub fn has_header(&self, name: &str) -> bool {
        self.headers.iter().any(|(n, _)| n.eq_ignore_ascii_case(name))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    /// Lower-case names.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

pub trait Transport: Send + Sync {
    /// Sends `request`; any HTTP status is a successful exchange, only I/O fails.
    fn send(&self, request: &Request) -> Result<Response>;
}

/// Job logs can be large; anything past this is cut off.
const MAX_BODY: u64 = 64 * 1024 * 1024;

/// The real transport: `ureq` with rustls.
#[derive(Debug)]
pub struct HttpTransport {
    agent: ureq::Agent,
}

impl Default for HttpTransport {
    fn default() -> Self {
        let config = ureq::Agent::config_builder()
            .max_redirects(0)
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_global(Some(Duration::from_secs(60)))
            .user_agent(concat!("power-idea/", env!("CARGO_PKG_VERSION")))
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
        }
    }
}

impl Transport for HttpTransport {
    fn send(&self, request: &Request) -> Result<Response> {
        let mut builder = ureq::http::Request::builder()
            .method(request.method)
            .uri(&request.url);
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        let host = request.url.split('/').nth(2).unwrap_or_default().to_string();
        // GET must go out without a body (and without `Content-Length: 0`).
        let sent = match &request.body {
            Some(body) => self
                .agent
                .run(builder.body(body.clone()).context("building the request")?),
            None => self.agent.run(builder.body(()).context("building the request")?),
        };
        // The error text names the host only: URLs of signed log links carry credentials.
        let mut response = sent
            .map_err(|err| anyhow::anyhow!("{}", describe(&err)))
            .with_context(|| format!("cannot reach {host}"))?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .map(|(n, v)| {
                (
                    n.as_str().to_ascii_lowercase(),
                    v.to_str().unwrap_or_default().to_string(),
                )
            })
            .collect();
        let body = response
            .body_mut()
            .with_config()
            .limit(MAX_BODY)
            .read_to_vec()
            .map_err(|err| anyhow::anyhow!("{}", describe(&err)))
            .with_context(|| format!("reading the response from {host}"))?;
        Ok(Response {
            status,
            headers,
            body,
        })
    }
}

fn describe(err: &ureq::Error) -> &'static str {
    match err {
        ureq::Error::Timeout(_) => "the request timed out",
        ureq::Error::HostNotFound => "host not found (offline?)",
        ureq::Error::ConnectionFailed => "connection failed",
        ureq::Error::Io(_) => "network error",
        ureq::Error::Tls(_) | ureq::Error::Rustls(_) => "TLS error",
        ureq::Error::BodyExceedsLimit(_) => "the response is too large",
        _ => "HTTP error",
    }
}

/// A scripted transport for tests: answers from a queue and records every request.
#[derive(Debug, Default)]
pub struct FakeTransport {
    responses: Mutex<VecDeque<Response>>,
    requests: Mutex<Vec<Request>>,
}

impl FakeTransport {
    pub fn push(&self, status: u16, headers: &[(&str, &str)], body: &str) {
        let response = Response {
            status,
            headers: headers
                .iter()
                .map(|(n, v)| (n.to_ascii_lowercase(), (*v).to_string()))
                .collect(),
            body: body.as_bytes().to_vec(),
        };
        if let Ok(mut queue) = self.responses.lock() {
            queue.push_back(response);
        }
    }

    pub fn requests(&self) -> Vec<Request> {
        self.requests.lock().map_or_else(|_| Vec::new(), |r| r.clone())
    }
}

impl Transport for FakeTransport {
    fn send(&self, request: &Request) -> Result<Response> {
        if let Ok(mut requests) = self.requests.lock() {
            requests.push(request.clone());
        }
        self.responses
            .lock()
            .ok()
            .and_then(|mut q| q.pop_front())
            .context("FakeTransport: no response queued")
    }
}
