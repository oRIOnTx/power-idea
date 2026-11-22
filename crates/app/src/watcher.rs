//! Classifies file system events of a project folder, so the tree, search
//! index and clean editors can follow changes made outside the IDE
//! (terminal, `git pull`, AI agents).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use notify::EventKind;
use notify::event::ModifyKind;

/// Folders whose churn never affects the tree or open files we care about.
const NOISY_DIRS: &[&str] = &[".git", "target", "node_modules", ".power-idea", "tmp", "log"];

/// What a batch of events changed.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Changes {
    /// Files or folders were created, removed or renamed.
    pub structure: bool,
    /// Files whose content may have changed.
    pub files: BTreeSet<PathBuf>,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        !self.structure && self.files.is_empty()
    }

    /// Adds one event; paths outside `root`, in noisy folders or excluded are ignored.
    pub fn add(&mut self, root: &Path, exclude: &[String], kind: EventKind, paths: &[PathBuf]) {
        let paths: Vec<&PathBuf> = paths.iter().filter(|p| !ignored(root, exclude, p)).collect();
        if paths.is_empty() {
            return;
        }
        match kind {
            EventKind::Access(_) => {}
            EventKind::Modify(ModifyKind::Data(_) | ModifyKind::Metadata(_)) => {
                self.files.extend(paths.into_iter().cloned());
            }
            // Create/remove/rename, and FSEvents' coarse "something changed".
            _ => {
                self.structure = true;
                self.files.extend(paths.into_iter().cloned());
            }
        }
    }
}

fn ignored(root: &Path, exclude: &[String], path: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(root) else {
        return true;
    };
    let first = rel.components().next().map(|c| c.as_os_str().to_string_lossy());
    if first.is_some_and(|f| NOISY_DIRS.contains(&f.as_ref())) {
        return true;
    }
    exclude.iter().any(|ex| rel.starts_with(ex))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use notify::EventKind;
    use notify::event::{CreateKind, DataChange, ModifyKind};

    use super::Changes;

    #[test]
    fn classifies_events() {
        let root = Path::new("/p");
        let exclude = vec!["vendor".to_string()];
        let mut changes = Changes::default();
        let edit = EventKind::Modify(ModifyKind::Data(DataChange::Content));
        changes.add(root, &exclude, edit, &[PathBuf::from("/p/src/a.rs")]);
        assert!(!changes.structure);
        assert!(changes.files.contains(Path::new("/p/src/a.rs")));

        // Noise, exclusions and paths outside the project are dropped.
        let create = EventKind::Create(CreateKind::File);
        for path in ["/p/.git/index", "/p/target/x", "/p/vendor/gem.rb", "/other/b.rs"] {
            changes.add(root, &exclude, create, &[PathBuf::from(path)]);
        }
        assert!(!changes.structure);

        changes.add(root, &exclude, create, &[PathBuf::from("/p/new.rs")]);
        assert!(changes.structure && changes.files.len() == 2);
    }
}
