//! `claude/settings.toml`: every Claude Code feature is an independent toggle, and the whole
//! section is off by default (see `schemas/settings.schema.json`).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};

/// How a feature reaches Claude Code.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Passed with CLI flags when the IDE starts `claude`; nothing outside the IDE changes.
    #[default]
    Inject,
    /// Written into `~/.claude` (managed, reversible) so it also applies outside the IDE.
    Install,
}

/// When code quality is measured.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunOn {
    /// After Claude edits a file (PostToolUse hook).
    #[default]
    Hook,
    /// When a file is saved in the IDE.
    Save,
    /// Only from the Run button.
    Manual,
}

pub const PERMISSION_MODES: &[&str] = &[
    "acceptEdits",
    "auto",
    "bypassPermissions",
    "manual",
    "dontAsk",
    "plan",
];
pub const EFFORT_LEVELS: &[&str] = &["low", "medium", "high", "xhigh", "max"];

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ClaudeSettings {
    /// The whole Claude Code section; off means the IDE behaves as without it.
    pub enabled: bool,
    pub general: General,
    pub rules: RulesSettings,
    pub skills: SkillsSettings,
    pub quality: QualitySettings,
    pub gauges: GaugeSettings,
    pub terminal: TerminalSettings,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct General {
    /// Path to `claude`; empty = found on PATH.
    pub claude_path: String,
    /// `--model` (alias or full name); empty = Claude Code's default.
    pub model: String,
    /// `--permission-mode`; empty = Claude Code's default.
    pub permission_mode: String,
    /// `--effort`; empty = Claude Code's default.
    pub effort: String,
    /// Give Claude the IDE's MCP server (`--mcp-config`) so it sees open files and diagnostics.
    pub ide_mcp: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RulesSettings {
    pub enabled: bool,
    pub mode: Mode,
}

impl Default for RulesSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: Mode::Inject,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SkillsSettings {
    pub enabled: bool,
    pub mode: Mode,
    /// Skill folder names that are switched off.
    pub disabled: Vec<String>,
}

impl Default for SkillsSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: Mode::Inject,
            disabled: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct QualitySettings {
    pub enabled: bool,
    pub run_on: RunOn,
    /// Score = 100 − error_weight × errors − warning_weight × warnings (clamped to 0…100).
    pub error_weight: f64,
    pub warning_weight: f64,
}

impl Default for QualitySettings {
    fn default() -> Self {
        Self {
            enabled: false,
            run_on: RunOn::Hook,
            error_weight: 5.0,
            warning_weight: 1.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)] // one switch per gauge and place, as in the UI
pub struct GaugeSettings {
    pub enabled: bool,
    pub context: bool,
    pub cost: bool,
    pub activity: bool,
    pub files: bool,
    pub quality: bool,
    pub in_status_bar: bool,
    /// Ignored: the Claude tab header shows only the tab name now. Still read so settings
    /// files written before stay valid under `deny_unknown_fields`; never written back.
    #[serde(skip_serializing)]
    in_tab_header: bool,
    pub in_sidebar: bool,
    /// Context usage turns amber at this percentage …
    pub context_amber: u8,
    /// … and red at this one.
    pub context_red: u8,
}

impl Default for GaugeSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            context: true,
            cost: true,
            activity: true,
            files: true,
            quality: true,
            in_status_bar: true,
            in_tab_header: false,
            in_sidebar: false,
            context_amber: 60,
            context_red: 85,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TerminalSettings {
    /// A pinned "Claude" tab first in the terminal panel.
    pub claude_tab: bool,
    /// Start `claude` as soon as the tab opens (else on click).
    pub auto_start: bool,
    /// `--continue` the last session in the project instead of a new one.
    pub continue_last: bool,
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            claude_tab: true,
            auto_start: true,
            continue_last: false,
        }
    }
}

impl ClaudeSettings {
    pub fn parse(text: &str) -> Result<Self> {
        let settings: Self = toml::from_str(text)?;
        settings.validate()?;
        Ok(settings)
    }

    /// Checks what serde cannot: allowed values and consistent thresholds.
    pub fn validate(&self) -> Result<()> {
        let general = &self.general;
        if !general.permission_mode.is_empty()
            && !PERMISSION_MODES.contains(&general.permission_mode.as_str())
        {
            bail!(
                "general.permission_mode: {:?} is not one of {}",
                general.permission_mode,
                PERMISSION_MODES.join(", ")
            );
        }
        if !general.effort.is_empty() && !EFFORT_LEVELS.contains(&general.effort.as_str()) {
            bail!(
                "general.effort: {:?} is not one of {}",
                general.effort,
                EFFORT_LEVELS.join(", ")
            );
        }
        let gauges = &self.gauges;
        if gauges.context_amber >= gauges.context_red || gauges.context_red > 100 {
            bail!("gauges: context_amber must be below context_red, and context_red at most 100");
        }
        let quality = &self.quality;
        if !(quality.error_weight >= 0.0 && quality.warning_weight >= 0.0) {
            bail!("quality: error_weight and warning_weight must be 0 or more");
        }
        Ok(())
    }

    /// The file text, with a `#:schema` line so editors (and agents) can validate it.
    pub fn render(&self) -> Result<String> {
        let body = toml::to_string_pretty(self).context("serializing settings")?;
        Ok(format!(
            "#:schema ./{SETTINGS_SCHEMA_FILE}\n\
             # Power IDEA → Claude Code settings. Edit here or in Settings → Claude Code;\n\
             # the IDE reloads this file when it changes.\n\n{body}"
        ))
    }
}

pub const SETTINGS_FILE: &str = "settings.toml";
pub const SETTINGS_SCHEMA_FILE: &str = "settings.schema.json";
pub const RULE_SCHEMA_FILE: &str = "rule.schema.json";
const SETTINGS_SCHEMA: &str = include_str!("../schemas/settings.schema.json");
const RULE_SCHEMA: &str = include_str!("../schemas/rule.schema.json");

/// The IDE's Claude Code folder (`<config>/claude`) and what lives in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeDir {
    pub root: PathBuf,
}

impl ClaudeDir {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn settings(&self) -> PathBuf {
        self.root.join(SETTINGS_FILE)
    }

    pub fn rules(&self) -> PathBuf {
        self.root.join("rules")
    }

    pub fn skills(&self) -> PathBuf {
        self.root.join("skills")
    }

    /// Creates the folders and (re)writes the schema files, which the IDE owns.
    pub fn ensure_layout(&self) -> Result<()> {
        fs::create_dir_all(self.rules()).context("creating the rules folder")?;
        fs::create_dir_all(self.skills()).context("creating the skills folder")?;
        write_if_changed(&self.root.join(SETTINGS_SCHEMA_FILE), SETTINGS_SCHEMA)?;
        write_if_changed(&self.rules().join(RULE_SCHEMA_FILE), RULE_SCHEMA)?;
        Ok(())
    }

    /// Reads `settings.toml`: defaults when missing, an error (with line and column) when invalid.
    pub fn load_settings(&self) -> Result<ClaudeSettings> {
        let path = self.settings();
        match fs::read_to_string(&path) {
            Ok(text) => ClaudeSettings::parse(&text).with_context(|| path.display().to_string()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(ClaudeSettings::default()),
            Err(err) => Err(err).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn save_settings(&self, settings: &ClaudeSettings) -> Result<()> {
        settings.validate()?;
        fs::create_dir_all(&self.root)?;
        write_if_changed(&self.settings(), &settings.render()?)
    }
}

/// Writes only when the content differs, so file watchers do not see no-op changes.
fn write_if_changed(path: &Path, text: &str) -> Result<()> {
    if fs::read_to_string(path).is_ok_and(|current| current == text) {
        return Ok(());
    }
    fs::write(path, text).with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn defaults_keep_the_section_off() {
        let settings = ClaudeSettings::parse("").unwrap();
        assert!(!settings.enabled);
        assert_eq!(settings, ClaudeSettings::default());
    }

    #[test]
    fn parses_and_round_trips() {
        let text = "enabled = true\n[general]\nmodel = \"opus\"\npermission_mode = \"plan\"\n\
                    [skills]\ndisabled = [\"tdd\"]\n[gauges]\ncontext_amber = 50\n";
        let settings = ClaudeSettings::parse(text).unwrap();
        assert!(settings.enabled);
        assert_eq!(settings.general.model, "opus");
        assert_eq!(settings.skills.disabled, ["tdd"]);
        assert_eq!(settings.gauges.context_amber, 50);
        assert!(settings.terminal.claude_tab, "unset sections keep their defaults");
        let rendered = settings.render().unwrap();
        assert!(rendered.starts_with("#:schema ./settings.schema.json\n"));
        assert_eq!(ClaudeSettings::parse(&rendered).unwrap(), settings);
    }

    #[test]
    fn reports_errors_with_location() {
        let err = |text: &str| format!("{:#}", ClaudeSettings::parse(text).unwrap_err());
        let unknown = err("enabled = true\n[gauges]\nno_such_gauge = true\n");
        assert!(
            unknown.contains("unknown field `no_such_gauge`") && unknown.contains("line 3"),
            "{unknown}"
        );
        assert!(err("[general]\npermission_mode = \"yolo\"").contains("not one of"));
        assert!(err("[general]\neffort = \"huge\"").contains("general.effort"));
        assert!(err("[gauges]\ncontext_amber = 90\ncontext_red = 80").contains("context_amber"));
        assert!(err("[quality]\nerror_weight = -1.0").contains("error_weight"));
        assert!(err("[rules]\nmode = \"everywhere\"").contains("unknown variant"));
    }

    #[test]
    fn accepts_the_removed_tab_header_switch() {
        let settings = ClaudeSettings::parse("[gauges]\nin_tab_header = true\n").unwrap();
        let written = toml::to_string(&settings).unwrap();
        assert_eq!(written, toml::to_string(&ClaudeSettings::default()).unwrap());
        assert!(!written.contains("in_tab_header"));
    }

    /// Every settings field is described in the schema and the schema names no extra fields
    /// (deprecated ones aside).
    #[test]
    fn schema_matches_the_settings() {
        let schema: Value = serde_json::from_str(SETTINGS_SCHEMA).unwrap();
        let defaults = serde_json::to_value(ClaudeSettings::default()).unwrap();
        assert_same_keys(&schema, &defaults, "settings");
    }

    fn assert_same_keys(schema: &Value, value: &Value, at: &str) {
        let Some(object) = value.as_object() else { return };
        let properties = schema["properties"]
            .as_object()
            .unwrap_or_else(|| panic!("{at}: no properties"));
        let mut expected: Vec<&String> = object.keys().collect();
        // Deprecated keys are still accepted but never written back.
        let mut described: Vec<&String> = properties
            .iter()
            .filter(|(_, p)| p["deprecated"] != Value::Bool(true))
            .map(|(k, _)| k)
            .collect();
        expected.sort();
        described.sort();
        assert_eq!(described, expected, "schema properties of {at}");
        assert_eq!(
            schema["additionalProperties"],
            Value::Bool(false),
            "{at} must reject unknown keys"
        );
        for (key, child) in object {
            assert_same_keys(&properties[key], child, &format!("{at}.{key}"));
        }
    }

    #[test]
    fn rule_schema_names_the_front_matter_fields() {
        let schema: Value = serde_json::from_str(RULE_SCHEMA).unwrap();
        let mut keys: Vec<&String> = schema["properties"].as_object().unwrap().keys().collect();
        keys.sort();
        assert_eq!(
            keys,
            ["applies_to", "description", "enabled", "name", "priority", "tags"]
        );
    }

    #[test]
    fn creates_the_layout_and_saves() {
        let dir = ClaudeDir::new(std::env::temp_dir().join(format!("pi-claude-{}", std::process::id())));
        let _ = fs::remove_dir_all(&dir.root);
        dir.ensure_layout().unwrap();
        assert!(dir.rules().join(RULE_SCHEMA_FILE).is_file());
        assert_eq!(
            dir.load_settings().unwrap(),
            ClaudeSettings::default(),
            "missing file = defaults"
        );
        let settings = ClaudeSettings {
            enabled: true,
            ..ClaudeSettings::default()
        };
        dir.save_settings(&settings).unwrap();
        assert_eq!(dir.load_settings().unwrap(), settings);
        fs::write(dir.settings(), "enabled = maybe").unwrap();
        assert!(format!("{:#}", dir.load_settings().unwrap_err()).contains("settings.toml"));
        let _ = fs::remove_dir_all(&dir.root);
    }
}
