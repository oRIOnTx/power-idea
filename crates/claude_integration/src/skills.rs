//! The skills library: standard `skills/<name>/SKILL.md` folders in the IDE's Claude folder,
//! curated built-ins seeded once (then the user's to edit), and the session plugin that carries
//! the enabled ones to Claude Code with `--plugin-dir`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use crate::frontmatter;

pub const SKILL_FILE: &str = "SKILL.md";
/// The generated plugin's name; its skills are invoked as `/power-idea:<skill>`.
pub const PLUGIN_NAME: &str = "power-idea";
/// Marks a skills folder whose built-ins were seeded, so deleting one does not bring it back.
const SEEDED_MARKER: &str = ".builtins-seeded";

const BUILTINS: &[(&str, &str)] = &[
    (
        "code-review",
        include_str!("../builtin/skills/code-review/SKILL.md"),
    ),
    (
        "write-tests-first",
        include_str!("../builtin/skills/write-tests-first/SKILL.md"),
    ),
    (
        "refactor-safely",
        include_str!("../builtin/skills/refactor-safely/SKILL.md"),
    ),
    (
        "conventional-commits",
        include_str!("../builtin/skills/conventional-commits/SKILL.md"),
    ),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    /// The folder name, which is what settings refer to.
    pub name: String,
    pub description: String,
    /// The skill folder.
    pub dir: PathBuf,
    /// Shipped with the IDE (it may have been edited since).
    pub builtin: bool,
}

/// The fields we read; SKILL.md has many more (allowed-tools, model, …), all passed through.
#[derive(Deserialize)]
struct SkillMeta {
    #[serde(default)]
    description: String,
}

/// A skill folder that could not be read, with the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillError {
    pub dir: PathBuf,
    pub message: String,
}

/// Writes the built-in skills into `dir` the first time only.
pub fn seed_builtins(dir: &Path) -> Result<()> {
    let marker = dir.join(SEEDED_MARKER);
    if marker.exists() {
        return Ok(());
    }
    for (name, text) in BUILTINS {
        let skill_dir = dir.join(name);
        if skill_dir.exists() {
            continue;
        }
        fs::create_dir_all(&skill_dir)?;
        fs::write(skill_dir.join(SKILL_FILE), text).with_context(|| format!("writing {name}"))?;
    }
    fs::write(
        &marker,
        "Built-in skills were copied here once; delete a folder to drop one.\n",
    )?;
    Ok(())
}

/// Reads every `<dir>/<name>/SKILL.md`, sorted by name.
pub fn load_dir(dir: &Path) -> (Vec<Skill>, Vec<SkillError>) {
    let mut skills = Vec::new();
    let mut errors = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return (skills, errors);
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.join(SKILL_FILE).is_file())
        .collect();
    dirs.sort();
    for skill_dir in dirs {
        match read_skill(&skill_dir) {
            Ok(skill) => skills.push(skill),
            Err(err) => errors.push(SkillError {
                dir: skill_dir,
                message: format!("{err:#}"),
            }),
        }
    }
    (skills, errors)
}

fn read_skill(dir: &Path) -> Result<Skill> {
    let name = dir
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    validate_name(&name)?;
    let text = fs::read_to_string(dir.join(SKILL_FILE)).context("reading SKILL.md")?;
    let meta: SkillMeta =
        serde_json::from_value(frontmatter::to_json(frontmatter::split(&text).front_matter)?)
            .context("SKILL.md front-matter")?;
    Ok(Skill {
        builtin: BUILTINS.iter().any(|(builtin, _)| *builtin == name),
        name,
        description: meta.description,
        dir: dir.to_path_buf(),
    })
}

/// Claude Code refuses names with `<`, `>`, control or zero-width characters and reserves a few.
pub fn validate_name(name: &str) -> Result<()> {
    let bad =
        |c: char| c == '<' || c == '>' || c.is_control() || matches!(c, '\u{200b}'..='\u{200f}' | '\u{feff}');
    if name.is_empty() || name.chars().any(bad) || name.contains('/') {
        bail!("invalid skill name {name:?}");
    }
    if name == "synced" || name.starts_with("anthropic-skills") {
        bail!("skill name {name:?} is reserved by Claude Code");
    }
    Ok(())
}

/// The enabled skills: all of `skills` except the names in `disabled`.
pub fn enabled<'a>(skills: &'a [Skill], disabled: &[String]) -> Vec<&'a Skill> {
    skills.iter().filter(|s| !disabled.contains(&s.name)).collect()
}

/// Writes the session plugin (`<plugin_dir>/.claude-plugin/plugin.json` + `skills/…`) holding
/// copies of `skill_dirs`, replacing what was there. Returns the folder for `--plugin-dir`.
pub fn write_plugin(plugin_dir: &Path, skill_dirs: &[PathBuf]) -> Result<PathBuf> {
    if plugin_dir.exists() {
        fs::remove_dir_all(plugin_dir).with_context(|| format!("clearing {}", plugin_dir.display()))?;
    }
    fs::create_dir_all(plugin_dir.join(".claude-plugin"))?;
    let manifest = serde_json::json!({
        "name": PLUGIN_NAME,
        "description": "Skills enabled in Power IDEA (Settings → Claude Code → Skills).",
    });
    fs::write(
        plugin_dir.join(".claude-plugin/plugin.json"),
        serde_json::to_string_pretty(&manifest)?,
    )?;
    for dir in skill_dirs {
        let Some(name) = dir.file_name() else { continue };
        copy_dir(dir, &plugin_dir.join("skills").join(name))?;
    }
    Ok(plugin_dir.to_path_buf())
}

/// Copies a folder recursively (skills may carry scripts and references next to SKILL.md).
pub fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from).with_context(|| format!("reading {}", from.display()))? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target).with_context(|| format!("copying {}", entry.path().display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pi-skills-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn seeds_builtins_once() {
        let dir = temp("seed");
        seed_builtins(&dir).unwrap();
        let (skills, errors) = load_dir(&dir);
        assert!(errors.is_empty(), "{errors:?}");
        let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "code-review",
                "conventional-commits",
                "refactor-safely",
                "write-tests-first"
            ]
        );
        assert!(skills.iter().all(|s| s.builtin && !s.description.is_empty()));

        fs::remove_dir_all(dir.join("code-review")).unwrap();
        seed_builtins(&dir).unwrap();
        assert!(
            !dir.join("code-review").exists(),
            "a deleted built-in stays deleted"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn writes_the_session_plugin() {
        let dir = temp("plugin");
        let library = dir.join("skills");
        fs::create_dir_all(library.join("mine/scripts")).unwrap();
        fs::write(
            library.join("mine/SKILL.md"),
            "---\nname: mine\ndescription: Mine\n---\nDo it.",
        )
        .unwrap();
        fs::write(library.join("mine/scripts/run.sh"), "echo hi").unwrap();
        fs::create_dir_all(library.join("off")).unwrap();
        fs::write(library.join("off/SKILL.md"), "---\ndescription: Off\n---\n").unwrap();

        let (skills, _) = load_dir(&library);
        let enabled: Vec<PathBuf> = enabled(&skills, &["off".to_string()])
            .iter()
            .map(|s| s.dir.clone())
            .collect();
        let plugin = write_plugin(&dir.join("cache/plugin"), &enabled).unwrap();
        let manifest = fs::read_to_string(plugin.join(".claude-plugin/plugin.json")).unwrap();
        assert!(manifest.contains("\"name\": \"power-idea\""));
        assert!(plugin.join("skills/mine/scripts/run.sh").is_file());
        assert!(!plugin.join("skills/off").exists());

        // Regenerating replaces the previous contents.
        write_plugin(&plugin, &[]).unwrap();
        assert!(!plugin.join("skills/mine").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn validates_names() {
        assert!(validate_name("code-review").is_ok());
        assert!(validate_name("bad<name>").is_err());
        assert!(validate_name("synced").is_err());
        assert!(validate_name("anthropic-skills-x").is_err());
        assert!(validate_name("").is_err());
    }
}
