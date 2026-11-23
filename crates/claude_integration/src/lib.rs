//! Claude Code integration (no UI): global settings, rules and skills in the IDE's config
//! folder, resolving them for a project, launching `claude`, installing globally, and the
//! hook/status-line bridge. Verified CLI facts: docs/claude-code-integration.md.

pub mod frontmatter;
pub mod project;
pub mod rules;
pub mod settings;

pub use project::ProjectFacts;
pub use rules::{AppliesTo, Rule, RuleMeta, Scope};
pub use settings::{ClaudeDir, ClaudeSettings, Mode};
