//! `claude/settings.toml`: the switches of the Claude Code section, off by default (see
//! `schemas/settings.schema.json`). Removed features (rules, skills, model, permissions, live
//! gauges) are listed in `REMOVED_KEYS`, which keeps files written before loadable.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

/// Keys of removed features. Old files still load: these are dropped before parsing and reported
/// on the page; any other unknown key is still an error.
pub const REMOVED_KEYS: &[&str] = &[
    "rules",
    "skills",
    "quality",
    "terminal",
    "general.model",
    "general.permission_mode",
    "general.effort",
    "gauges",
];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ClaudeSettings {
    /// The whole Claude Code section; off means the IDE starts `claude` as a terminal would.
    pub enabled: bool,
    pub general: General,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct General {
    /// Path to `claude`; empty = found on PATH.
    pub claude_path: String,
    /// Give Claude the IDE's MCP server (`--mcp-config`) so it sees open files and diagnostics.
    pub ide_mcp: bool,
}

/// Settings read from a file, plus the removed keys the file still has.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Loaded {
    pub settings: ClaudeSettings,
    pub removed_keys: Vec<String>,
}

impl ClaudeSettings {
    pub fn parse(text: &str) -> Result<Self> {
        Ok(Self::load(text)?.settings)
    }

    /// Parses a file, dropping (and reporting) the keys of removed features.
    pub fn load(text: &str) -> Result<Loaded> {
        let mut table: toml::Table = toml::from_str(text)?;
        let removed_keys = strip_removed(&mut table);
        // Parsing the text itself keeps line and column in errors; a stripped table has none.
        let settings: Self = if removed_keys.is_empty() {
            toml::from_str(text)?
        } else {
            toml::Value::Table(table).try_into()?
        };
        Ok(Loaded {
            settings,
            removed_keys,
        })
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

/// Removes `REMOVED_KEYS` from `table`, returning those that were there.
fn strip_removed(table: &mut toml::Table) -> Vec<String> {
    REMOVED_KEYS
        .iter()
        .filter(|key| {
            let removed = match key.split_once('.') {
                Some((section, field)) => table
                    .get_mut(section)
                    .and_then(toml::Value::as_table_mut)
                    .and_then(|section| section.remove(field)),
                None => table.remove(**key),
            };
            removed.is_some()
        })
        .map(|key| (*key).to_string())
        .collect()
}

pub const SETTINGS_FILE: &str = "settings.toml";
pub const SETTINGS_SCHEMA_FILE: &str = "settings.schema.json";
const SETTINGS_SCHEMA: &str = include_str!("../schemas/settings.schema.json");

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

    /// Creates the folder and (re)writes the schema file, which the IDE owns.
    pub fn ensure_layout(&self) -> Result<()> {
        fs::create_dir_all(&self.root).context("creating the Claude settings folder")?;
        write_if_changed(&self.root.join(SETTINGS_SCHEMA_FILE), SETTINGS_SCHEMA)
    }

    /// Reads `settings.toml`: defaults when missing, an error (with line and column) when invalid.
    pub fn load_settings(&self) -> Result<Loaded> {
        let path = self.settings();
        match fs::read_to_string(&path) {
            Ok(text) => ClaudeSettings::load(&text).with_context(|| path.display().to_string()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Loaded::default()),
            Err(err) => Err(err).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn save_settings(&self, settings: &ClaudeSettings) -> Result<()> {
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
        let text = "enabled = true\n[general]\nide_mcp = true\n";
        let settings = ClaudeSettings::parse(text).unwrap();
        assert!(settings.enabled && settings.general.ide_mcp);
        assert!(
            settings.general.claude_path.is_empty(),
            "unset keys keep their defaults"
        );
        let rendered = settings.render().unwrap();
        assert!(rendered.starts_with("#:schema ./settings.schema.json\n"));
        assert_eq!(ClaudeSettings::parse(&rendered).unwrap(), settings);
    }

    #[test]
    fn reports_errors_with_location() {
        let err = |text: &str| format!("{:#}", ClaudeSettings::parse(text).unwrap_err());
        let unknown = err("enabled = true\n[general]\nno_such_key = true\n");
        assert!(
            unknown.contains("unknown field `no_such_key`") && unknown.contains("line 3"),
            "{unknown}"
        );
    }

    #[test]
    fn old_files_load_without_the_removed_keys() {
        let text = "enabled = true\n[general]\nclaude_path = \"/bin/claude\"\nmodel = \"opus\"\n\
                    permission_mode = \"auto\"\neffort = \"medium\"\n[rules]\nenabled = true\n\
                    [skills]\ndisabled = []\n[quality]\nenabled = false\n[terminal]\nclaude_tab = false\n\
                    [gauges]\nquality = true\nin_tab_header = false\ncontext_red = 90\n";
        let loaded = ClaudeSettings::load(text).unwrap();
        assert_eq!(loaded.removed_keys, REMOVED_KEYS);
        assert!(loaded.settings.enabled);
        assert_eq!(loaded.settings.general.claude_path, "/bin/claude");
        let rendered = loaded.settings.render().unwrap();
        assert!(ClaudeSettings::load(&rendered).unwrap().removed_keys.is_empty());
    }

    #[test]
    fn unknown_keys_stay_errors_next_to_removed_ones() {
        let err =
            ClaudeSettings::load("[rules]\nenabled = true\n[general]\nno_such_key = true\n").unwrap_err();
        assert!(format!("{err:#}").contains("no_such_key"), "{err:#}");
        assert!(ClaudeSettings::load("[general]\nmodel_name = \"x\"\n").is_err());
    }

    /// Every settings field is described in the schema and the schema names no extra fields.
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
        let mut described: Vec<&String> = properties.keys().collect();
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
    fn creates_the_layout_and_saves() {
        let dir = ClaudeDir::new(std::env::temp_dir().join(format!("pi-claude-{}", std::process::id())));
        let _ = fs::remove_dir_all(&dir.root);
        dir.ensure_layout().unwrap();
        assert!(dir.root.join(SETTINGS_SCHEMA_FILE).is_file());
        assert_eq!(
            dir.load_settings().unwrap(),
            Loaded::default(),
            "missing file = defaults"
        );
        let settings = ClaudeSettings {
            enabled: true,
            ..ClaudeSettings::default()
        };
        dir.save_settings(&settings).unwrap();
        assert_eq!(dir.load_settings().unwrap().settings, settings);
        fs::write(dir.settings(), "enabled = maybe").unwrap();
        assert!(format!("{:#}", dir.load_settings().unwrap_err()).contains("settings.toml"));
        let _ = fs::remove_dir_all(&dir.root);
    }
}
