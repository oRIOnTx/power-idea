//! What the Claude Code integration needs from a project window.

use claude_integration::ProjectFacts;

use super::Workspace;

impl Workspace {
    /// The project's root, files and languages, for matching rules.
    pub(crate) fn claude_facts(&self) -> ProjectFacts {
        ProjectFacts::new(self.root.clone(), self.index.files().to_vec())
    }
}
