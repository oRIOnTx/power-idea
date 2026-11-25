//! GitHub Actions REST client: conditional requests (ETag → 304, free of rate limit),
//! rate-limit tracking, and job logs whose redirect never carries the token.

use std::collections::HashMap;
use std::fmt::{self, Write as _};
use std::sync::{Arc, Mutex};

use serde::de::DeserializeOwned;
use serde_json::json;

use crate::auth::Token;
use crate::models::{Annotation, Job, JobList, Run, RunList, Workflow, WorkflowList};
use crate::remote::RepoId;
use crate::time::Millis;
use crate::transport::{Request, Response, Transport};

pub const API: &str = "https://api.github.com";
const API_VERSION: &str = "2022-11-28";
/// The local clock and GitHub's may disagree by a few seconds.
const CLOCK_SLACK: Millis = 30_000;

/// What `x-ratelimit-*` said on the latest response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimit {
    pub remaining: u32,
    pub limit: u32,
    /// Unix seconds when the window resets.
    pub reset_at: i64,
}

impl RateLimit {
    fn from_response(response: &Response) -> Option<Self> {
        let num = |name: &str| response.header(name)?.trim().parse::<i64>().ok();
        Some(Self {
            remaining: u32::try_from(num("x-ratelimit-remaining")?).ok()?,
            limit: u32::try_from(num("x-ratelimit-limit").unwrap_or(0)).unwrap_or(0),
            reset_at: num("x-ratelimit-reset").unwrap_or(0),
        })
    }
}

/// A user-facing failure; `Display` says what happened and how to fix it.
#[derive(Debug)]
pub enum ApiError {
    /// 401: the token is missing, expired or revoked.
    Unauthorized,
    /// 403 that is not a rate limit: the token lacks a permission or SSO is not authorized.
    Forbidden(String),
    /// 404: no such repository for this token (private repo without access), run or log.
    NotFound,
    RateLimited {
        /// Unix seconds.
        reset_at: Option<i64>,
    },
    Http {
        status: u16,
        message: String,
    },
    Network(anyhow::Error),
    /// The response body was not what the API documents.
    Decode(String),
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unauthorized => write!(
                f,
                "GitHub rejected the token (401). Log in again with `gh auth login` or save a new token."
            ),
            Self::Forbidden(message) => write!(
                f,
                "GitHub denied access (403): {message}. The token needs Actions: read & write, \
                 Contents: read and Metadata: read."
            ),
            Self::NotFound => write!(
                f,
                "Not found (404). The repository may be private and the token has no access to it."
            ),
            Self::RateLimited { .. } => write!(f, "GitHub API rate limit reached; waiting for it to reset."),
            Self::Http { status, message } => write!(f, "GitHub answered {status}: {message}"),
            Self::Network(err) => write!(f, "{err:#}"),
            Self::Decode(what) => write!(f, "Unexpected response from GitHub ({what})"),
        }
    }
}

impl std::error::Error for ApiError {}

impl ApiError {
    /// Worth recording in the error journal: not an expected state like "no access".
    pub fn is_unexpected(&self) -> bool {
        matches!(self, Self::Http { .. } | Self::Decode(_))
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

/// A value and whether it differs from the previous fetch (false after a 304).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched<T> {
    pub value: T,
    pub changed: bool,
}

/// Which runs to list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunFilter {
    pub branch: Option<String>,
    pub workflow_id: Option<u64>,
    /// `queued`, `in_progress`, `completed`, `success`, `failure`, …
    pub status: Option<String>,
    pub per_page: u32,
}

struct Cached {
    etag: String,
    body: Vec<u8>,
}

struct Inner {
    transport: Arc<dyn Transport>,
    token: Token,
    repo: RepoId,
    api: String,
    cache: Mutex<HashMap<String, Cached>>,
    rate: Mutex<Option<RateLimit>>,
}

/// Cheap to clone; safe to use from background threads.
#[derive(Clone)]
pub struct GitHub {
    inner: Arc<Inner>,
}

impl fmt::Debug for GitHub {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GitHub")
            .field("repo", &self.inner.repo)
            .finish_non_exhaustive()
    }
}

impl GitHub {
    pub fn new(transport: Arc<dyn Transport>, token: Token, repo: RepoId) -> Self {
        Self::with_api(transport, token, repo, API)
    }

    pub fn with_api(transport: Arc<dyn Transport>, token: Token, repo: RepoId, api: &str) -> Self {
        Self {
            inner: Arc::new(Inner {
                transport,
                token,
                repo,
                api: api.trim_end_matches('/').to_string(),
                cache: Mutex::new(HashMap::new()),
                rate: Mutex::new(None),
            }),
        }
    }

    pub fn repo(&self) -> &RepoId {
        &self.inner.repo
    }

    pub fn rate_limit(&self) -> Option<RateLimit> {
        self.inner.rate.lock().ok().and_then(|r| *r)
    }

    pub fn workflows(&self) -> ApiResult<Fetched<Vec<Workflow>>> {
        let fetched =
            self.get_json::<WorkflowList>(&format!("{}/actions/workflows?per_page=100", self.base()))?;
        Ok(Fetched {
            value: fetched.value.workflows,
            changed: fetched.changed,
        })
    }

    pub fn runs(&self, filter: &RunFilter) -> ApiResult<Fetched<Vec<Run>>> {
        let fetched = self.get_json::<RunList>(&self.runs_url(filter))?;
        Ok(Fetched {
            value: fetched.value.workflow_runs,
            changed: fetched.changed,
        })
    }

    /// One run (for runs started from the IDE that are not in the listed page).
    pub fn run(&self, run_id: u64) -> ApiResult<Fetched<Run>> {
        self.get_json::<Run>(&format!("{}/actions/runs/{run_id}", self.base()))
    }

    pub fn jobs(&self, run_id: u64) -> ApiResult<Fetched<Vec<Job>>> {
        let url = format!("{}/actions/runs/{run_id}/jobs?per_page=100", self.base());
        let fetched = self.get_json::<JobList>(&url)?;
        Ok(Fetched {
            value: fetched.value.jobs,
            changed: fetched.changed,
        })
    }

    /// Error annotations of a job (for Actions, the job id is the check run id).
    pub fn annotations(&self, job_id: u64) -> ApiResult<Vec<Annotation>> {
        let url = format!("{}/check-runs/{job_id}/annotations?per_page=100", self.base());
        Ok(self.get_json::<Vec<Annotation>>(&url)?.value)
    }

    /// The plain-text log of a finished job. The API answers with a redirect to a short-lived
    /// signed URL on another host, which is fetched without the Authorization header.
    pub fn job_log(&self, job_id: u64) -> ApiResult<String> {
        let url = format!("{}/actions/jobs/{job_id}/logs", self.base());
        let response = self.send(self.authorized(Request::get(url)))?;
        let response = match response.status {
            301 | 302 | 303 | 307 | 308 => {
                let location = response
                    .header("location")
                    .ok_or_else(|| ApiError::Decode("redirect without a location".into()))?
                    .to_string();
                let followed = self
                    .inner
                    .transport
                    .send(&Request::get(location))
                    .map_err(ApiError::Network)?;
                if followed.status != 200 {
                    return Err(status_error(&followed));
                }
                followed
            }
            200 => response,
            _ => return Err(status_error(&response)),
        };
        Ok(String::from_utf8_lossy(&response.body).into_owned())
    }

    /// Re-runs every job of a finished run.
    pub fn rerun(&self, run_id: u64) -> ApiResult<()> {
        self.post(
            &format!("{}/actions/runs/{run_id}/rerun", self.base()),
            &json!({}),
        )
        .map(drop)
    }

    /// Re-runs the failed jobs of a run (and the jobs that depend on them).
    pub fn rerun_failed(&self, run_id: u64) -> ApiResult<()> {
        self.post(
            &format!("{}/actions/runs/{run_id}/rerun-failed-jobs", self.base()),
            &json!({}),
        )
        .map(drop)
    }

    pub fn cancel(&self, run_id: u64) -> ApiResult<()> {
        self.post(
            &format!("{}/actions/runs/{run_id}/cancel", self.base()),
            &json!({}),
        )
        .map(drop)
    }

    /// Starts a `workflow_dispatch` run on `git_ref`. Returns the run id when GitHub says it.
    pub fn dispatch(
        &self,
        workflow_id: u64,
        git_ref: &str,
        inputs: serde_json::Map<String, serde_json::Value>,
    ) -> ApiResult<Option<u64>> {
        let url = format!("{}/actions/workflows/{workflow_id}/dispatches", self.base());
        let response = self.post(&url, &json!({ "ref": git_ref, "inputs": inputs }))?;
        Ok(serde_json::from_slice::<serde_json::Value>(&response.body)
            .ok()
            .and_then(|v| v.get("workflow_run_id")?.as_u64()))
    }

    /// The run a dispatch created: the newest `workflow_dispatch` run of the workflow on `branch`
    /// created at or after `since` (Unix millis, with some slack for clock skew).
    pub fn find_dispatched_run(
        &self,
        workflow_id: u64,
        branch: &str,
        since: Millis,
    ) -> ApiResult<Option<Run>> {
        let url = format!(
            "{}/actions/workflows/{workflow_id}/runs?event=workflow_dispatch&per_page=5&branch={}",
            self.base(),
            encode_query_value(branch)
        );
        let runs = self.get_json::<RunList>(&url)?.value.workflow_runs;
        Ok(runs
            .into_iter()
            .filter(|r| r.created_millis().is_some_and(|t| t >= since - CLOCK_SLACK))
            .max_by_key(|r| r.id))
    }

    /// A file of the repository at `git_ref` (empty: the default branch), for workflow YAML
    /// that is not checked out locally.
    pub fn file(&self, path: &str, git_ref: &str) -> ApiResult<String> {
        let mut url = format!(
            "{}/contents/{}",
            self.base(),
            path.split('/')
                .map(encode_query_value)
                .collect::<Vec<_>>()
                .join("/"),
        );
        if !git_ref.is_empty() {
            url.push_str("?ref=");
            url.push_str(&encode_query_value(git_ref));
        }
        let request = self.authorized(Request::get(url).header("Accept", "application/vnd.github.raw+json"));
        let response = self.send(request)?;
        if response.status != 200 {
            return Err(status_error(&response));
        }
        Ok(String::from_utf8_lossy(&response.body).into_owned())
    }

    fn post(&self, url: &str, body: &serde_json::Value) -> ApiResult<Response> {
        let response = self.send(self.authorized(Request::post_json(url, body)))?;
        match response.status {
            200..=299 => Ok(response),
            _ => Err(status_error(&response)),
        }
    }

    fn base(&self) -> String {
        format!("{}/repos/{}", self.inner.api, self.inner.repo)
    }

    fn runs_url(&self, filter: &RunFilter) -> String {
        let path = match filter.workflow_id {
            Some(id) => format!("{}/actions/workflows/{id}/runs", self.base()),
            None => format!("{}/actions/runs", self.base()),
        };
        let per_page = if filter.per_page == 0 {
            20
        } else {
            filter.per_page.min(100)
        };
        let mut url = format!("{path}?per_page={per_page}");
        if let Some(branch) = &filter.branch {
            url.push_str("&branch=");
            url.push_str(&encode_query_value(branch));
        }
        if let Some(status) = &filter.status {
            url.push_str("&status=");
            url.push_str(&encode_query_value(status));
        }
        url
    }

    fn authorized(&self, request: Request) -> Request {
        let request = if request.has_header("Accept") {
            request
        } else {
            request.header("Accept", "application/vnd.github+json")
        };
        request
            .header("Authorization", format!("Bearer {}", self.inner.token.expose()))
            .header("X-GitHub-Api-Version", API_VERSION)
    }

    fn send(&self, request: Request) -> ApiResult<Response> {
        let response = self.inner.transport.send(&request).map_err(ApiError::Network)?;
        if let Some(rate) = RateLimit::from_response(&response)
            && let Ok(mut slot) = self.inner.rate.lock()
        {
            *slot = Some(rate);
        }
        Ok(response)
    }

    fn get_json<T: DeserializeOwned>(&self, url: &str) -> ApiResult<Fetched<T>> {
        let mut request = self.authorized(Request::get(url));
        let cached_etag = self
            .inner
            .cache
            .lock()
            .ok()
            .and_then(|c| c.get(url).map(|c| c.etag.clone()));
        if let Some(etag) = &cached_etag {
            request = request.header("If-None-Match", etag.clone());
        }
        let response = self.send(request)?;
        let (body, changed) = match response.status {
            304 => {
                let body = self
                    .inner
                    .cache
                    .lock()
                    .ok()
                    .and_then(|c| c.get(url).map(|c| c.body.clone()))
                    .ok_or_else(|| ApiError::Decode("304 without a cached copy".into()))?;
                (body, false)
            }
            200 => {
                if let Some(etag) = response.header("etag")
                    && let Ok(mut cache) = self.inner.cache.lock()
                {
                    cache.insert(
                        url.to_string(),
                        Cached {
                            etag: etag.to_string(),
                            body: response.body.clone(),
                        },
                    );
                }
                (response.body, true)
            }
            _ => return Err(status_error(&response)),
        };
        let value = serde_json::from_slice(&body).map_err(|err| ApiError::Decode(err.to_string()))?;
        Ok(Fetched { value, changed })
    }
}

fn status_error(response: &Response) -> ApiError {
    let message = serde_json::from_slice::<serde_json::Value>(&response.body)
        .ok()
        .and_then(|v| v.get("message")?.as_str().map(str::to_string))
        .unwrap_or_else(|| "no details".into());
    let reset_at = response
        .header("x-ratelimit-reset")
        .and_then(|v| v.trim().parse().ok());
    let exhausted = response
        .header("x-ratelimit-remaining")
        .is_some_and(|v| v.trim() == "0");
    match response.status {
        401 => ApiError::Unauthorized,
        429 => ApiError::RateLimited { reset_at },
        403 if exhausted || response.header("retry-after").is_some() => ApiError::RateLimited { reset_at },
        403 => ApiError::Forbidden(message),
        404 | 410 => ApiError::NotFound,
        status => ApiError::Http { status, message },
    }
}

/// Percent-encodes a query value (branch names may contain `/`, `#`, `&`, `+`, …).
fn encode_query_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~' | b'/') {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

#[cfg(test)]
mod tests;
