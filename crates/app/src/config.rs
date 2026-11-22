//! User configuration persisted as TOML in the platform config dir
//! (`~/Library/Application Support/power-idea/config.toml` on macOS).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

const MAX_RECENT_PROJECTS: usize = 20;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    /// Neo Dark.
    #[default]
    Dark,
    /// Neo Light.
    Light,
    /// A lighter dark theme in the style of current JetBrains IDEs.
    Graphite,
}

impl Appearance {
    pub fn is_dark(self) -> bool {
        self != Self::Light
    }
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
    /// Where newly opened projects go when a project window is open.
    pub open_project_mode: crate::projects::OpenProjectMode,
    /// Shell for integrated terminals; empty means `$SHELL`.
    pub terminal_shell: String,
    /// Bottom tool windows (Terminal, Git Log…) span the whole window, below the
    /// Project tree, as in JetBrains IDEs; otherwise they sit under the editor only.
    pub bottom_panel_full_width: bool,
    /// Language servers keyed by id (e.g. "ruby"), merged over the built-in list.
    pub language_servers: BTreeMap<String, LanguageServerConfig>,
    pub recent_projects: Vec<PathBuf>,
    /// Project windows open when the app last quit, reopened at launch; empty means the
    /// Welcome screen. `None` in configs written before this was recorded.
    pub open_projects: Option<Vec<PathBuf>>,
    /// Claude Session Insights (reads ~/.claude transcripts locally); off by default.
    pub insights_enabled: bool,
    /// Ask before ⌘Q quits (unsaved files are always asked about).
    pub confirm_exit: bool,
    /// Ask before a project window closes.
    pub confirm_close_project: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LanguageServerConfig {
    pub enabled: bool,
    pub command: String,
    pub args: Vec<String>,
    /// Editor languages (as returned by `editor::canonical_language`) it serves.
    pub languages: Vec<String>,
    /// Shown in Settings when the command is not installed.
    pub install_hint: String,
}

impl Default for LanguageServerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            command: String::new(),
            args: Vec::new(),
            languages: Vec::new(),
            install_hint: String::new(),
        }
    }
}

fn server(command: &str, args: &[&str], languages: &[&str], install_hint: &str) -> LanguageServerConfig {
    LanguageServerConfig {
        enabled: true,
        command: command.into(),
        args: args.iter().map(|a| a.to_string()).collect(),
        languages: languages.iter().map(|l| l.to_string()).collect(),
        install_hint: install_hint.into(),
    }
}

/// Built-in language servers; users can override or add more in config.toml.
pub fn default_language_servers() -> BTreeMap<String, LanguageServerConfig> {
    BTreeMap::from([
        (
            "ruby".into(),
            server("ruby-lsp", &[], &["ruby"], "gem install ruby-lsp"),
        ),
        (
            "rust".into(),
            server(
                "rust-analyzer",
                &[],
                &["rust"],
                "rustup component add rust-analyzer",
            ),
        ),
        (
            "typescript".into(),
            server(
                "typescript-language-server",
                &["--stdio"],
                &["typescript", "javascript", "tsx"],
                "npm install -g typescript-language-server typescript",
            ),
        ),
        (
            "python".into(),
            server(
                "pyright-langserver",
                &["--stdio"],
                &["python"],
                "npm install -g pyright",
            ),
        ),
        (
            "go".into(),
            server(
                "gopls",
                &[],
                &["go"],
                "go install golang.org/x/tools/gopls@latest",
            ),
        ),
    ])
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
            open_project_mode: Default::default(),
            terminal_shell: String::new(),
            bottom_panel_full_width: true,
            language_servers: default_language_servers(),
            recent_projects: Vec::new(),
            open_projects: None,
            insights_enabled: false,
            confirm_exit: true,
            confirm_close_project: true,
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
        let mut config: Self = Self::path()
            .and_then(|path| fs::read_to_string(path).ok())
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default();
        // A user table replaces the whole map, so bring back missing built-ins.
        for (id, server) in default_language_servers() {
            config.language_servers.entry(id).or_insert(server);
        }
        config
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

    /// Projects to open at launch: those open when the app last quit (none → Welcome);
    /// for older configs, the most recent project.
    pub fn startup_projects(&self) -> Vec<PathBuf> {
        match &self.open_projects {
            Some(open) => open.clone(),
            None => self.recent_projects.iter().take(1).cloned().collect(),
        }
    }
}

/// Per-project settings in `<project>/.power-idea/settings.toml`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectSettings {
    /// Overrides the global tab size for this project.
    pub tab_size: Option<usize>,
    /// Paths relative to the project root hidden from search and Go to File.
    pub exclude: Vec<String>,
}

impl ProjectSettings {
    fn path(root: &Path) -> PathBuf {
        root.join(".power-idea").join("settings.toml")
    }

    pub fn load(root: &Path) -> Self {
        fs::read_to_string(Self::path(root))
            .ok()
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        let path = Self::path(root);
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(&path, toml::to_string_pretty(self)?).with_context(|| format!("writing {}", path.display()))
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
    fn launch_reopens_what_was_open_at_quit() {
        let mut config = Config::default();
        config.touch_recent_project(Path::new("/a"));
        assert_eq!(config.startup_projects(), [PathBuf::from("/a")]);
        // Quit from the Welcome screen: nothing reopens, even with recent projects.
        config.open_projects = Some(Vec::new());
        assert!(config.startup_projects().is_empty());
        config.open_projects = Some(vec![PathBuf::from("/b"), PathBuf::from("/c")]);
        assert_eq!(
            config.startup_projects(),
            [PathBuf::from("/b"), PathBuf::from("/c")]
        );
        let saved: Config = toml::from_str(&toml::to_string(&config).unwrap()).unwrap();
        assert_eq!(saved.open_projects, config.open_projects);
    }

    #[test]
    fn user_language_servers_merge_with_builtins() {
        let text =
            "[language_servers.ruby]\ncommand = \"solargraph\"\nargs = [\"stdio\"]\nlanguages = [\"ruby\"]\n";
        let mut config: Config = toml::from_str(text).unwrap();
        for (id, server) in default_language_servers() {
            config.language_servers.entry(id).or_insert(server);
        }
        assert_eq!(config.language_servers["ruby"].command, "solargraph");
        assert!(config.language_servers["ruby"].enabled);
        assert!(config.language_servers.contains_key("rust"));
    }

    #[test]
    fn partial_toml_uses_defaults() {
        let config: Config = toml::from_str("appearance = \"graphite\"").unwrap();
        assert!(config.appearance.is_dark());
        let config: Config = toml::from_str("appearance = \"light\"").unwrap();
        assert_eq!(config.appearance, Appearance::Light);
        assert_eq!(config.tab_size, 4);
    }
}
