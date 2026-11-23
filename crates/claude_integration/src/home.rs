//! Claude Code's own files that Settings → Claude Code opens for editing: the user settings and
//! the global CLAUDE.md in `~/.claude`. The IDE only creates a missing file on request, empty.

use std::fs;
use std::io::{ErrorKind, Write as _};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

/// `~/.claude/settings.json`: model, permissions, hooks and the rest of Claude Code's settings.
pub fn user_settings(claude_home: &Path) -> PathBuf {
    claude_home.join("settings.json")
}

/// `~/.claude/CLAUDE.md`: instructions Claude Code loads in every project.
pub fn global_rules(claude_home: &Path) -> PathBuf {
    claude_home.join("CLAUDE.md")
}

/// What a missing file starts as: the smallest valid content, nothing of the IDE's own.
pub fn initial_text(path: &Path) -> &'static str {
    if path.extension().is_some_and(|ext| ext == "json") {
        "{}\n"
    } else {
        ""
    }
}

/// Creates `path` (and its folder) with `initial_text` when it does not exist; an existing file
/// is never touched. Returns whether it was created.
pub fn ensure_file(path: &Path) -> Result<bool> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    // `create_new` fails instead of truncating if the file appeared in the meantime.
    let mut file = match fs::OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == ErrorKind::AlreadyExists => return Ok(false),
        Err(err) => return Err(err).with_context(|| format!("creating {}", path.display())),
    };
    file.write_all(initial_text(path).as_bytes())
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_missing_files_only() {
        let home = std::env::temp_dir().join(format!("pi-claude-home-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        let settings = user_settings(&home);
        assert!(ensure_file(&settings).unwrap());
        assert_eq!(fs::read_to_string(&settings).unwrap(), "{}\n");
        let rules = global_rules(&home);
        assert!(ensure_file(&rules).unwrap());
        assert_eq!(fs::read_to_string(&rules).unwrap(), "");

        fs::write(&rules, "# Mine\n").unwrap();
        assert!(!ensure_file(&rules).unwrap(), "an existing file stays as it is");
        assert_eq!(fs::read_to_string(&rules).unwrap(), "# Mine\n");
        let _ = fs::remove_dir_all(&home);
    }
}
