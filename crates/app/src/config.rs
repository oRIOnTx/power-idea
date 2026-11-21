//! User configuration persisted as TOML in the platform config dir
//! (`~/Library/Application Support/power-idea/config.toml` on macOS).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

const MAX_RECENT_PROJECTS: usize = 20;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    #[default]
    Dark,
    Light,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub appearance: Appearance,
    pub editor_font_family: String,
    pub editor_font_size: f32,
    pub ui_font_size: f32,
    pub tab_size: usize,
    /// Local MCP server for Claude Code and other agents.
    pub mcp_enabled: bool,
    pub mcp_port: u16,
    pub recent_projects: Vec<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            appearance: Appearance::Dark,
            editor_font_family: "JetBrains Mono".into(),
            editor_font_size: 13.0,
            ui_font_size: 13.0,
            tab_size: 4,
            mcp_enabled: true,
            mcp_port: 7419,
            recent_projects: Vec::new(),
        }
    }
}

impl Config {
    pub fn dir() -> Option<PathBuf> {
        dirs::config_dir().map(|dir| dir.join("power-idea"))
    }

    fn path() -> Option<PathBuf> {
        Self::dir().map(|dir| dir.join("config.toml"))
    }

    /// Loads the config, falling back to defaults when missing or invalid.
    pub fn load() -> Self {
        Self::path()
            .and_then(|path| fs::read_to_string(path).ok())
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path().context("no config directory on this platform")?;
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(&path, toml::to_string_pretty(self)?).with_context(|| format!("writing {}", path.display()))
    }

    /// Moves `project` to the front of the recent list.
    pub fn touch_recent_project(&mut self, project: &Path) {
        self.recent_projects.retain(|p| p != project);
        self.recent_projects.insert(0, project.to_path_buf());
        self.recent_projects.truncate(MAX_RECENT_PROJECTS);
    }
}

/// The loaded config, shared by all windows.
pub struct AppConfig(pub Config);

impl gpui_kit::Global for AppConfig {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_projects_are_deduplicated_and_ordered() {
        let mut config = Config::default();
        config.touch_recent_project(Path::new("/a"));
        config.touch_recent_project(Path::new("/b"));
        config.touch_recent_project(Path::new("/a"));
        assert_eq!(config.recent_projects, [PathBuf::from("/a"), PathBuf::from("/b")]);
    }

    #[test]
    fn partial_toml_uses_defaults() {
        let config: Config = toml::from_str("appearance = \"light\"").unwrap();
        assert_eq!(config.appearance, Appearance::Light);
        assert_eq!(config.tab_size, 4);
    }
}
