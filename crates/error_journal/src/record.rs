//! The on-disk shapes: one `ErrorRecord` per unique error, inside a `JournalFile` header.

use serde::{Deserialize, Serialize};

use crate::text;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Panic,
    Error,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Panic => "Panic",
            Self::Error => "Error",
        }
    }
}

/// What the running build and machine look like; stamped on every record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub app_version: String,
    pub build: String,
    pub os: String,
    pub session_id: String,
    /// Replaced with `~` in everything stored.
    pub home: Option<String>,
}

/// A freshly captured error before it is redacted, trimmed and fingerprinted.
#[derive(Debug, Clone, Default)]
pub struct Capture {
    pub kind: Option<Kind>,
    pub message: String,
    pub location: Option<String>,
    pub thread: Option<String>,
    pub context: Option<String>,
    /// The full `Backtrace` text; trimmed when the record is built.
    pub backtrace: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorRecord {
    pub id: String,
    pub fingerprint: String,
    /// Unix seconds.
    pub first_seen: u64,
    pub last_seen: u64,
    pub count: u32,
    pub seen: bool,
    pub kind: Kind,
    pub message: String,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub thread: Option<String>,
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default)]
    pub backtrace: String,
    pub app_version: String,
    pub build: String,
    pub os: String,
    pub session_id: String,
    /// A panic from a session that did not shut down cleanly: kept by "Until restart"
    /// retention until the user has seen it.
    #[serde(default)]
    pub from_crash: bool,
}

/// Longest message kept; a panic message can embed whole documents.
const MAX_MESSAGE: usize = 4 * 1024;

impl ErrorRecord {
    /// Redacts, trims and fingerprints `capture`. `seq` makes the id unique within a session.
    pub fn build(capture: Capture, ident: &Identity, now: u64, seq: u64) -> Self {
        let home = ident.home.as_deref();
        let kind = capture.kind.unwrap_or(Kind::Error);
        let message = text::truncate(&text::redact(&capture.message, home), MAX_MESSAGE);
        let backtrace = text::redact(&text::trim_backtrace(&capture.backtrace), home);
        let frames = text::own_frames(&capture.backtrace);
        let fingerprint = text::fingerprint(kind, &message, &frames);
        let redact = |value: Option<String>| value.map(|v| text::redact(&v, home));
        Self {
            id: format!("{}-{seq}", ident.session_id),
            fingerprint,
            first_seen: now,
            last_seen: now,
            count: 1,
            seen: false,
            kind,
            message,
            location: redact(capture.location),
            thread: capture.thread,
            context: redact(capture.context),
            backtrace,
            app_version: ident.app_version.clone(),
            build: ident.build.clone(),
            os: ident.os.clone(),
            session_id: ident.session_id.clone(),
            from_crash: false,
        }
    }

    /// A Markdown block ready to paste into a GitHub issue.
    pub fn to_markdown(&self) -> String {
        use std::fmt::Write as _;
        let mut out = format!("### {}: {}\n\n", self.kind.label(), first_line(&self.message));
        let _ = writeln!(out, "- **Version:** {} (build {})", self.app_version, self.build);
        let _ = writeln!(out, "- **OS:** {}", self.os);
        let _ = writeln!(
            out,
            "- **Seen:** {}× · first {} · last {}",
            self.count,
            text::format_utc(self.first_seen),
            text::format_utc(self.last_seen)
        );
        for (label, value) in [
            ("Location", &self.location),
            ("Thread", &self.thread),
            ("Context", &self.context),
        ] {
            if let Some(value) = value {
                let _ = writeln!(out, "- **{label}:** `{value}`");
            }
        }
        if self.message.contains('\n') {
            let _ = write!(out, "\n```text\n{}\n```\n", self.message);
        }
        if !self.backtrace.is_empty() {
            let _ = write!(
                out,
                "\n<details><summary>Backtrace</summary>\n\n```text\n{}\n```\n</details>\n",
                self.backtrace.trim_end()
            );
        }
        out
    }
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or_default()
}

/// Every record as Markdown, newest first.
pub fn records_to_markdown(records: &[ErrorRecord]) -> String {
    records
        .iter()
        .map(ErrorRecord::to_markdown)
        .collect::<Vec<_>>()
        .join("\n---\n\n")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalFile {
    pub app_version: String,
    pub build: String,
    pub session_id: String,
    pub clean_shutdown: bool,
    #[serde(default)]
    pub records: Vec<ErrorRecord>,
}

impl JournalFile {
    pub fn fresh(ident: &Identity) -> Self {
        Self {
            app_version: ident.app_version.clone(),
            build: ident.build.clone(),
            session_id: ident.session_id.clone(),
            clean_shutdown: false,
            records: Vec::new(),
        }
    }

    /// The bytes written to disk; the size limit is measured on them.
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec_pretty(self).unwrap_or_default()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn ident() -> Identity {
        Identity {
            app_version: "1.0.0".into(),
            build: "10".into(),
            os: "macOS 26.1 (aarch64)".into(),
            session_id: "s2".into(),
            home: Some("/Users/alice".into()),
        }
    }

    pub fn capture(message: &str) -> Capture {
        Capture {
            kind: Some(Kind::Error),
            message: message.into(),
            ..Capture::default()
        }
    }

    #[test]
    fn markdown_has_version_os_and_backtrace() {
        let mut capture = capture("boom");
        capture.backtrace = "   0: power_idea::main\n             at ./crates/app/src/main.rs:1:1\n".into();
        capture.location = Some("crates/app/src/main.rs:1:1".into());
        let record = ErrorRecord::build(capture, &ident(), 0, 1);
        let md = record.to_markdown();
        assert!(md.starts_with("### Error: boom"));
        assert!(md.contains("**Version:** 1.0.0 (build 10)"));
        assert!(md.contains("**OS:** macOS 26.1"));
        assert!(md.contains("1970-01-01 00:00 UTC"));
        assert!(md.contains("power_idea::main"));
    }
}
