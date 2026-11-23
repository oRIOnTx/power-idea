//! One-time cleanup after the removed Install mode: takes out of `~/.claude` exactly what the IDE
//! once put there (the managed CLAUDE.md block and the `ide-*` skill copies), as recorded in its
//! install manifest. User-authored text and folders are never touched; old backups stay.

use std::fs;
use std::path::Path;

use anyhow::{Context as _, Result};
use serde::Deserialize;

pub const BLOCK_START: &str = "<!-- ide:managed:start -->";
pub const BLOCK_END: &str = "<!-- ide:managed:end -->";
/// Prefix of the skill folders the IDE managed in `~/.claude/skills`.
const SKILL_PREFIX: &str = "ide-";
/// Inside every managed skill copy, so a folder the user made is never taken for ours.
const SKILL_MARKER: &str = ".power-idea-managed";
/// Written by the Install mode into the IDE's Claude settings folder.
pub const MANIFEST_FILE: &str = "install-manifest.json";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Manifest {
    /// The CLAUDE.md block, when it was installed.
    claude_md: Option<FileRecord>,
    /// Managed skill folder names (`ide-<name>`) in `~/.claude/skills`.
    skills: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct FileRecord {
    /// The file did not exist before the IDE created it.
    created: bool,
    /// Text the IDE inserted before the block, removed with it.
    separator: String,
}

/// Removes what the manifest in `state_dir` lists from `claude_home`, then the manifest itself.
/// Returns what was removed, for one notice; empty when there was nothing to clean up.
pub fn cleanup(claude_home: &Path, state_dir: &Path) -> Result<Vec<String>> {
    let manifest_path = state_dir.join(MANIFEST_FILE);
    let text = match fs::read_to_string(&manifest_path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err).with_context(|| format!("reading {}", manifest_path.display())),
    };
    // An unreadable manifest is left in place: removing anything without it would be a guess.
    let manifest: Manifest =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", manifest_path.display()))?;
    let mut removed = Vec::new();
    if let Some(record) = &manifest.claude_md
        && remove_claude_md_block(&claude_home.join("CLAUDE.md"), record)?
    {
        removed.push("the IDE rules block in ~/.claude/CLAUDE.md".to_string());
    }
    for name in &manifest.skills {
        if remove_skill_copy(&claude_home.join("skills"), name)? {
            removed.push(format!("~/.claude/skills/{name}"));
        }
    }
    fs::remove_file(&manifest_path).with_context(|| format!("removing {}", manifest_path.display()))?;
    Ok(removed)
}

/// Takes the managed block out of `path`; a file the IDE created and that holds nothing else
/// is deleted. Returns whether the file changed.
fn remove_claude_md_block(path: &Path, record: &FileRecord) -> Result<bool> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err).with_context(|| format!("reading {}", path.display())),
    };
    let Some(restored) = remove_block(&text, &record.separator) else {
        return Ok(false);
    };
    if record.created && restored.is_empty() {
        fs::remove_file(path).with_context(|| format!("removing {}", path.display()))?;
    } else {
        fs::write(path, restored).with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(true)
}

/// Deletes `skills_dir/name` only when it is one of the IDE's copies (prefix and marker file).
fn remove_skill_copy(skills_dir: &Path, name: &str) -> Result<bool> {
    let plain_name = Path::new(name).file_name().is_some_and(|n| n == name);
    if !plain_name || !name.starts_with(SKILL_PREFIX) {
        return Ok(false);
    }
    let dir = skills_dir.join(name);
    if !dir.join(SKILL_MARKER).is_file() {
        return Ok(false);
    }
    fs::remove_dir_all(&dir).with_context(|| format!("removing {}", dir.display()))?;
    Ok(true)
}

/// `text` without the managed block (and without `separator`, the text the IDE put before a
/// block it appended at the end). `None` when there is no block.
fn remove_block(text: &str, separator: &str) -> Option<String> {
    let start = text.find(BLOCK_START)?;
    let end_marker = start + text.get(start..)?.find(BLOCK_END)?;
    let mut end = end_marker + BLOCK_END.len();
    if text.get(end..)?.starts_with('\n') {
        end += 1;
    }
    let (before, after) = (text.get(..start)?, text.get(end..)?);
    let before = if after.is_empty() {
        before.strip_suffix(separator).unwrap_or(before)
    } else {
        before
    };
    Some(format!("{before}{after}"))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pi-legacy-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The block exactly as the Install mode wrote it.
    fn block(content: &str) -> String {
        format!(
            "{BLOCK_START}\n<!-- Managed by Power IDEA (Settings → Claude Code). Edits inside this block are overwritten. -->\n{content}\n{BLOCK_END}\n"
        )
    }

    fn manifest(state: &Path, json: &str) {
        fs::create_dir_all(state).unwrap();
        fs::write(state.join(MANIFEST_FILE), json).unwrap();
    }

    #[test]
    fn nothing_to_do_without_a_manifest() {
        let root = temp("none");
        fs::write(root.join("CLAUDE.md"), block("Rules")).unwrap();
        assert!(cleanup(&root, &root.join("ide")).unwrap().is_empty());
        assert_eq!(
            fs::read_to_string(root.join("CLAUDE.md")).unwrap(),
            block("Rules")
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn restores_the_user_text_and_removes_managed_skills() {
        let root = temp("restore");
        let (home, state) = (root.join(".claude"), root.join("ide"));
        fs::create_dir_all(home.join("skills/ide-review")).unwrap();
        fs::create_dir_all(home.join("skills/mine")).unwrap();
        fs::write(home.join("skills/ide-review/SKILL.md"), "ide").unwrap();
        fs::write(home.join("skills/ide-review").join(SKILL_MARKER), "").unwrap();
        fs::write(home.join("skills/mine/SKILL.md"), "user skill").unwrap();
        let original = "# My global rules\n\nBe brief.";
        let installed = format!("{original}\n\n{}\n# Added later by the user\n", block("Rules"));
        fs::write(home.join("CLAUDE.md"), installed).unwrap();
        manifest(
            &state,
            r#"{"claude_md":{"created":false,"separator":"\n\n"},"skills":["ide-review"]}"#,
        );

        let removed = cleanup(&home, &state).unwrap();
        assert_eq!(removed.len(), 2, "{removed:?}");
        assert_eq!(
            fs::read_to_string(home.join("CLAUDE.md")).unwrap(),
            format!("{original}\n\n\n# Added later by the user\n")
        );
        assert!(!home.join("skills/ide-review").exists());
        assert!(home.join("skills/mine/SKILL.md").is_file());
        assert!(!state.join(MANIFEST_FILE).exists(), "runs once");
        assert!(cleanup(&home, &state).unwrap().is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn appended_block_comes_out_byte_identical() {
        let root = temp("append");
        let (home, state) = (root.join(".claude"), root.join("ide"));
        fs::create_dir_all(&home).unwrap();
        let original = "# Mine, no newline";
        fs::write(home.join("CLAUDE.md"), format!("{original}\n\n{}", block("R"))).unwrap();
        manifest(
            &state,
            r#"{"claude_md":{"created":false,"separator":"\n\n"},"skills":[]}"#,
        );
        cleanup(&home, &state).unwrap();
        assert_eq!(fs::read_to_string(home.join("CLAUDE.md")).unwrap(), original);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn deletes_a_claude_md_the_ide_created() {
        let root = temp("created");
        let (home, state) = (root.join(".claude"), root.join("ide"));
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join("CLAUDE.md"), block("Rules")).unwrap();
        manifest(
            &state,
            r#"{"claude_md":{"created":true,"separator":""},"skills":[]}"#,
        );
        assert_eq!(cleanup(&home, &state).unwrap().len(), 1);
        assert!(!home.join("CLAUDE.md").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn never_touches_folders_the_ide_does_not_own() {
        let root = temp("foreign");
        let (home, state) = (root.join(".claude"), root.join("ide"));
        fs::create_dir_all(home.join("skills/ide-review")).unwrap();
        fs::write(home.join("skills/ide-review/SKILL.md"), "mine").unwrap();
        fs::create_dir_all(home.join("skills/review")).unwrap();
        fs::write(home.join("skills/review").join(SKILL_MARKER), "").unwrap();
        manifest(
            &state,
            r#"{"skills":["ide-review","review","../skills/review","ide-../../x"]}"#,
        );
        assert!(cleanup(&home, &state).unwrap().is_empty());
        assert!(
            home.join("skills/ide-review/SKILL.md").is_file(),
            "no marker: the user's"
        );
        assert!(home.join("skills/review").is_dir(), "no prefix: not ours");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn keeps_an_unreadable_manifest() {
        let root = temp("broken");
        manifest(&root, "{not json");
        assert!(cleanup(&root, &root).is_err());
        assert!(root.join(MANIFEST_FILE).is_file());
        let _ = fs::remove_dir_all(&root);
    }
}
