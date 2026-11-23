//! Unsaved editor text kept when the app quits without asking (Dock → Quit,
//! logout), restored as unsaved edits the next time the project opens.
//! Stored per project in `~/Library/Application Support/power-idea/recovery/`.

use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context as _, Result, bail};

const MANIFEST: &str = "files.txt";

/// Stable short name for a project folder (FNV-1a of its path).
pub(crate) fn project_key(root: &Path) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in root.to_string_lossy().bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Project-relative paths only: no absolute paths or `..` escaping the project.
fn safe_rel(rel: &str) -> Result<&Path> {
    let path = Path::new(rel);
    if rel.is_empty() || path.components().any(|c| !matches!(c, Component::Normal(_))) {
        bail!("unsafe recovery path {rel}");
    }
    Ok(path)
}

pub struct Recovery {
    base: PathBuf,
}

impl Recovery {
    /// Recovery storage under the app's config folder.
    pub fn new() -> Option<Self> {
        crate::config::Config::dir().map(|dir| Self::at(dir.join("recovery")))
    }

    pub fn at(base: PathBuf) -> Self {
        Self { base }
    }

    fn dir(&self, root: &Path) -> PathBuf {
        self.base.join(project_key(root))
    }

    /// Replaces the saved text of `root` with `files` (project-relative path, text).
    pub fn store(&self, root: &Path, files: &[(String, String)]) -> Result<()> {
        let dir = self.dir(root);
        let _ = fs::remove_dir_all(&dir);
        if files.is_empty() {
            return Ok(());
        }
        let mut manifest = String::new();
        for (ix, (rel, text)) in files.iter().enumerate() {
            safe_rel(rel)?;
            fs::create_dir_all(&dir).context("creating the recovery folder")?;
            fs::write(dir.join(format!("{ix}.txt")), text).context("writing recovery text")?;
            manifest.push_str(rel);
            manifest.push('\n');
        }
        fs::write(dir.join(MANIFEST), manifest).context("writing the recovery manifest")
    }

    /// Returns and removes the saved text of `root`.
    pub fn take(&self, root: &Path) -> Vec<(String, String)> {
        let dir = self.dir(root);
        let Ok(manifest) = fs::read_to_string(dir.join(MANIFEST)) else {
            return Vec::new();
        };
        let files = manifest
            .lines()
            .enumerate()
            .filter(|(_, rel)| safe_rel(rel).is_ok())
            .filter_map(|(ix, rel)| {
                Some((
                    rel.to_string(),
                    fs::read_to_string(dir.join(format!("{ix}.txt"))).ok()?,
                ))
            })
            .collect();
        let _ = fs::remove_dir_all(&dir);
        files
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{Recovery, project_key};

    #[test]
    fn stores_and_takes_unsaved_text_once() {
        let base = std::env::temp_dir().join(format!("power-idea-recovery-{}", std::process::id()));
        let recovery = Recovery::at(base.clone());
        let root = Path::new("/p/app");
        let files = vec![
            ("src/main.rs".to_string(), "fn main() {}\n".to_string()),
            ("README.md".to_string(), "# draft".to_string()),
        ];
        recovery.store(root, &files).unwrap();
        assert!(recovery.take(Path::new("/p/other")).is_empty());
        assert_eq!(recovery.take(root), files);
        assert!(recovery.take(root).is_empty(), "taken only once");

        assert!(recovery.store(root, &[("../escape".into(), "x".into())]).is_err());
        assert!(
            recovery
                .store(root, &[("/etc/hosts".into(), "x".into())])
                .is_err()
        );
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn project_keys_are_stable_and_distinct() {
        assert_eq!(project_key(Path::new("/p/app")), project_key(Path::new("/p/app")));
        assert_ne!(
            project_key(Path::new("/p/app")),
            project_key(Path::new("/p/app2"))
        );
    }
}
