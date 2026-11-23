//! Claude Code integration (no UI): global settings, rules and skills in the IDE's config
//! folder, resolving them for a project, launching `claude`, installing globally, and the
//! hook/status-line bridge. Verified CLI facts: docs/claude-code-integration.md.

pub mod frontmatter;
pub mod installer;
pub mod launcher;
pub mod project;
pub mod resolver;
pub mod rules;
pub mod settings;
pub mod skills;

pub use project::ProjectFacts;
pub use resolver::{Effective, resolve};
pub use rules::{AppliesTo, Rule, RuleMeta, Scope};
pub use settings::{ClaudeDir, ClaudeSettings, Mode};
pub use skills::Skill;
