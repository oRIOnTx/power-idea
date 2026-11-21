//! Per-project workspace state restored when a project is reopened: open tabs
//! with cursors, expanded folders, recent files and tool windows. Stored in
//! `<project>/.power-idea/workspace.toml`, which is personal and git-ignored.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

const FILE: &str = "workspace.toml";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    pub tabs: Vec<SessionTab>,
    /// Index into `tabs`.
    pub active: Option<usize>,
    pub expanded: Vec<String>,
    pub recent_files: Vec<String>,
    pub show_sidebar: bool,
    /// `project`, `commit` or `database`.
    pub sidebar: String,
    pub git_log_open: bool,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            tabs: Vec::new(),
            active: None,
            expanded: Vec::new(),
            recent_files: Vec::new(),
            show_sidebar: true,
            sidebar: "project".into(),
            git_log_open: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionTab {
    /// Relative to the project root.
    pub path: String,
    /// 0-based cursor position.
    #[serde(default)]
    pub line: u32,
    #[serde(default)]
    pub column: u32,
    /// Preview mode of Markdown/SVG tabs: `editor`, `split` or `preview`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
}

fn dir(root: &Path) -> PathBuf {
    root.join(".power-idea")
}

impl Session {
    /// Loads the saved state; a missing or broken file gives the default.
    pub fn load(root: &Path) -> Self {
        let Ok(text) = fs::read_to_string(dir(root).join(FILE)) else {
            return Self::default();
        };
        toml::from_str(&text).unwrap_or_else(|err| {
            eprintln!("{FILE}: {err}");
            Self::default()
        })
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        let dir = dir(root);
        fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let gitignore = dir.join(".gitignore");
        let existing = fs::read_to_string(&gitignore).unwrap_or_default();
        if let Some(text) = gitignore_with(&existing, FILE) {
            fs::write(&gitignore, text).context("updating .power-idea/.gitignore")?;
        }
        let text = toml::to_string_pretty(self).context("serializing workspace state")?;
        fs::write(dir.join(FILE), text).context("writing workspace state")
    }

    /// Drops entries whose files or folders no longer exist.
    pub fn retain_existing(&mut self, root: &Path) {
        let active = self
            .active
            .and_then(|ix| self.tabs.get(ix))
            .map(|t| t.path.clone());
        self.tabs.retain(|t| root.join(&t.path).is_file());
        self.active = active.and_then(|path| self.tabs.iter().position(|t| t.path == path));
        self.expanded.retain(|p| root.join(p).is_dir());
        self.recent_files.retain(|p| root.join(p).is_file());
    }
}

/// `existing` .gitignore text with `entry` added, or `None` when it is already listed.
fn gitignore_with(existing: &str, entry: &str) -> Option<String> {
    if existing.lines().any(|line| line.trim() == entry) {
        return None;
    }
    let mut text = existing.to_string();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(entry);
    text.push('\n');
    Some(text)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{Session, SessionTab, gitignore_with};

    #[test]
    fn adds_gitignore_entry_once() {
        assert_eq!(gitignore_with("", "workspace.toml").unwrap(), "workspace.toml\n");
        assert_eq!(
            gitignore_with("a", "workspace.toml").unwrap(),
            "a\nworkspace.toml\n"
        );
        assert_eq!(gitignore_with("x\nworkspace.toml\n", "workspace.toml"), None);
    }

    #[test]
    fn saves_loads_and_prunes() {
        let root = std::env::temp_dir().join(format!("power-idea-session-{}", std::process::id()));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}").unwrap();

        let tab = |path: &str| SessionTab {
            path: path.into(),
            line: 3,
            column: 1,
            preview: None,
        };
        let mut session = Session {
            tabs: vec![tab("gone.rs"), tab("src/main.rs")],
            active: Some(1),
            expanded: vec!["src".into(), "old".into()],
            recent_files: vec!["src/main.rs".into(), "gone.rs".into()],
            git_log_open: true,
            ..Session::default()
        };
        session.save(&root).unwrap();
        session.save(&root).unwrap();
        let loaded = Session::load(&root);
        assert_eq!(loaded, session);
        assert_eq!(
            fs::read_to_string(root.join(".power-idea/.gitignore")).unwrap(),
            "workspace.toml\n"
        );

        session.retain_existing(&root);
        assert_eq!(session.tabs, [tab("src/main.rs")]);
        assert_eq!(session.active, Some(0));
        assert_eq!(session.expanded, ["src"]);
        assert_eq!(session.recent_files, ["src/main.rs"]);

        fs::write(root.join(".power-idea/workspace.toml"), "not = [valid").unwrap();
        assert_eq!(Session::load(&root), Session::default());
        fs::remove_dir_all(&root).unwrap();
    }
}
