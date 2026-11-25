//! GitHub Actions for the IDE without a UI: remote → repository, token, REST client with
//! conditional requests, poll scheduling and job-log parsing.

pub mod auth;
pub mod github;
pub mod log_parser;
pub mod models;
pub mod poller;
pub mod remote;
pub mod time;
pub mod transport;

pub use auth::{Token, TokenSource};
pub use github::{ApiError, Fetched, GitHub, RateLimit, RunFilter};
pub use log_parser::{LineKind, LogLine, ParsedLog, parse_log};
pub use models::{Annotation, Job, Run, State, Step, Workflow};
pub use remote::{RepoId, parse_github_remote};
