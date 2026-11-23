//! Tab name, pin state and working directory of a terminal tab.

use std::path::{Path, PathBuf};

use super::TerminalView;

impl TerminalView {
    /// The renamed title, else the shell's title (run tabs: their configuration name).
    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.title)
    }

    pub fn custom_name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// An empty (or blank) name restores the default title.
    pub fn set_name(&mut self, name: &str) {
        let name = name.trim();
        self.name = (!name.is_empty()).then(|| name.to_string());
    }

    pub fn is_pinned(&self) -> bool {
        self.pinned
    }

    /// Only plain shells can be pinned: run and Claude tabs belong to their command.
    pub fn can_pin(&self) -> bool {
        self.run.is_none()
    }

    pub fn set_pinned(&mut self, pinned: bool) {
        self.pinned = pinned && self.can_pin();
    }

    pub fn cwd(&self) -> Option<&Path> {
        self.cwd.as_deref()
    }

    pub fn set_cwd(&mut self, cwd: PathBuf) {
        self.cwd = Some(cwd);
    }

    /// Builder form for tabs created or restored in `cwd`.
    pub fn in_dir(mut self, cwd: PathBuf) -> Self {
        self.cwd = Some(cwd);
        self
    }
}
