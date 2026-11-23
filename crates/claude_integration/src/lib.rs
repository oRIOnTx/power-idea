//! Claude Code integration (no UI): the IDE's settings for it, launching `claude`, the
//! hook/status-line bridge with its live gauges, Claude Code's own files the settings page opens,
//! and the one-time cleanup of the removed Install mode. Verified CLI facts: docs/claude-code-integration.md.

pub mod bridge;
pub mod gauges;
pub mod home;
pub mod launcher;
pub mod legacy;
pub mod settings;

pub use settings::{ClaudeDir, ClaudeSettings};
