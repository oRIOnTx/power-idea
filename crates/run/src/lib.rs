//! Run configurations, independent of language: a shell command with a working
//! folder and environment. User configurations live in
//! `<project>/.power-idea/run.toml`; common ones are detected from the project
//! (Rails, RSpec, Cargo, npm/yarn/pnpm/bun, Go, pytest/Django, Mix, Make).

mod detect;
mod locations;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

pub use detect::detect;
pub use locations::{Location, find_locations};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunConfig {
    pub name: String,
    /// Shell command line, run by the user's login shell (so rbenv, nvm… work).
    pub command: String,
    /// Working folder relative to the project root (the root when empty).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// Found by `detect` rather than written by the user.
    #[serde(skip)]
    pub detected: bool,
}

impl RunConfig {
    pub fn new(name: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            command: command.into(),
            cwd: None,
            env: BTreeMap::new(),
            detected: true,
        }
    }

    /// Absolute working folder.
    pub fn working_dir(&self, root: &Path) -> PathBuf {
        match self.cwd.as_deref().filter(|c| !c.is_empty()) {
            Some(cwd) => root.join(cwd),
            None => root.to_path_buf(),
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
struct RunFile {
    #[serde(default, rename = "config")]
    configs: Vec<RunConfig>,
}

pub fn config_file(root: &Path) -> PathBuf {
    root.join(".power-idea").join("run.toml")
}

/// User configurations from `run.toml` (an empty list when there is none).
pub fn load(root: &Path) -> Result<Vec<RunConfig>> {
    let path = config_file(root);
    let Ok(text) = fs::read_to_string(&path) else {
        return Ok(Vec::new());
    };
    let file: RunFile = toml::from_str(&text).with_context(|| format!("reading {}", path.display()))?;
    Ok(file
        .configs
        .into_iter()
        .map(|c| RunConfig { detected: false, ..c })
        .collect())
}

/// User configurations first, then detected ones whose names are not taken.
pub fn all(root: &Path) -> Result<Vec<RunConfig>> {
    let mut configs = load(root)?;
    for detected in detect(root) {
        if !configs.iter().any(|c| c.name == detected.name) {
            configs.push(detected);
        }
    }
    Ok(configs)
}

/// Starting text for `run.toml`: the detected configurations as examples.
pub fn template(root: &Path) -> String {
    let mut text = String::from(
        "# Run configurations for Power IDEA (Run ⌃R, Stop ⌘F2).\n\
         # Each [[config]] runs `command` in your login shell; `cwd` is relative to the project.\n\
         # Detected configurations are listed automatically; copy one here to change it.\n\n",
    );
    let examples = detect(root);
    let sample = examples
        .first()
        .cloned()
        .unwrap_or_else(|| RunConfig::new("Example", "echo hello"));
    text.push_str("[[config]]\n");
    text.push_str(&format!(
        "name = {:?}\ncommand = {:?}\n",
        sample.name, sample.command
    ));
    text.push_str("# cwd = \"subfolder\"\n# env = { RAILS_ENV = \"development\" }\n");
    for other in examples.iter().skip(1) {
        text.push_str(&format!(
            "\n# [[config]]\n# name = {:?}\n# command = {:?}\n",
            other.name, other.command
        ));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::{RunConfig, all, config_file, load, template};

    #[test]
    fn user_configs_override_detected_ones() {
        let dir = std::env::temp_dir().join(format!("power-idea-run-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(".power-idea")).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        assert!(load(&dir).unwrap().is_empty());

        std::fs::write(
            config_file(&dir),
            "[[config]]\nname = \"cargo test\"\ncommand = \"cargo test -- --nocapture\"\ncwd = \"crates/a\"\nenv = { RUST_LOG = \"debug\" }\n",
        )
        .unwrap();
        let configs = all(&dir).unwrap();
        assert_eq!(configs[0].command, "cargo test -- --nocapture");
        assert!(!configs[0].detected);
        assert_eq!(configs[0].env["RUST_LOG"], "debug");
        assert_eq!(configs[0].working_dir(&dir), dir.join("crates/a"));
        assert_eq!(configs.iter().filter(|c| c.name == "cargo test").count(), 1);
        assert!(configs.iter().any(|c| c.name == "cargo run" && c.detected));

        // The template is valid TOML with the first detected config.
        let parsed: super::RunFile = toml::from_str(&template(&dir)).unwrap();
        assert_eq!(parsed.configs.len(), 1);
        std::fs::write(config_file(&dir), "not = [valid").unwrap();
        assert!(load(&dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            RunConfig::new("a", "b").working_dir(std::path::Path::new("/p")),
            std::path::Path::new("/p")
        );
    }
}
