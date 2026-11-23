//! Pinned terminal tabs, saved per project outside the repository in
//! `<config>/workspaces/<key>/pinned_terminals.json`: name, order and working directory.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};

const FILE_NAME: &str = "pinned_terminals.json";
const VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinnedTab {
    /// The name given with Rename; `None` keeps the default title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Relative to the project root when inside it, else absolute; `""` is the root.
    pub cwd: String,
    pub order: usize,
}

#[derive(Serialize, Deserialize)]
struct StoredFile {
    version: u32,
    tabs: Vec<PinnedTab>,
}

pub struct PinnedStore {
    path: PathBuf,
}

impl PinnedStore {
    /// The store of the project at `root`, keyed by its canonical path.
    pub fn for_project(root: &Path) -> Option<Self> {
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let dir = crate::config::Config::dir()?;
        Some(Self::at(
            dir.join("workspaces")
                .join(crate::recovery::project_key(&root))
                .join(FILE_NAME),
        ))
    }

    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    /// The saved tabs in order; a missing file means none, a broken one is an error.
    pub fn load(&self) -> Result<Vec<PinnedTab>> {
        let text = match fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err).with_context(|| format!("reading {}", self.path.display())),
        };
        let file: StoredFile =
            serde_json::from_str(&text).with_context(|| format!("parsing {}", self.path.display()))?;
        if file.version != VERSION {
            bail!("{}: unsupported version {}", self.path.display(), file.version);
        }
        let mut tabs = file.tabs;
        tabs.sort_by_key(|t| t.order);
        Ok(tabs)
    }

    /// Writes atomically (temp file + rename), so a crash never leaves half a file.
    pub fn save(&self, tabs: &[PinnedTab]) -> Result<()> {
        let dir = self.path.parent().context("pinned tabs path has no folder")?;
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        let file = StoredFile {
            version: VERSION,
            tabs: tabs.to_vec(),
        };
        let text = serde_json::to_string_pretty(&file)?;
        let temp = self.path.with_extension("json.tmp");
        fs::write(&temp, text).with_context(|| format!("writing {}", temp.display()))?;
        fs::rename(&temp, &self.path).with_context(|| format!("replacing {}", self.path.display()))
    }
}

/// `dir` relative to `root` when inside it, else absolute.
pub fn encode_cwd(root: &Path, dir: &Path) -> String {
    match dir.strip_prefix(root) {
        Ok(rel) => rel.to_string_lossy().into_owned(),
        Err(_) => dir.to_string_lossy().into_owned(),
    }
}

/// The saved directory, or its nearest existing parent, or the project root.
pub fn resolve_cwd(root: &Path, saved: &str) -> PathBuf {
    let path = if saved.is_empty() {
        root.to_path_buf()
    } else {
        root.join(saved)
    };
    path.ancestors()
        .find(|dir| dir.is_dir())
        .map_or_else(|| root.to_path_buf(), Path::to_path_buf)
}

/// Moves pinned items before the others, keeping the relative order of both groups.
pub fn pinned_first<T>(items: &mut Vec<T>, is_pinned: impl Fn(&T) -> bool) {
    let (mut pinned, rest): (Vec<T>, Vec<T>) = items.drain(..).partition(|item| is_pinned(item));
    pinned.extend(rest);
    *items = pinned;
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{PinnedStore, PinnedTab, encode_cwd, pinned_first, resolve_cwd};

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("power-idea-pins-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn tab(name: Option<&str>, cwd: &str, order: usize) -> PinnedTab {
        PinnedTab {
            name: name.map(str::to_string),
            cwd: cwd.into(),
            order,
        }
    }

    #[test]
    fn saves_and_loads_in_order() {
        let dir = temp_dir("round-trip");
        let store = PinnedStore::at(dir.join("ws/pinned_terminals.json"));
        assert_eq!(store.load().unwrap(), vec![], "missing file: no pinned tabs");
        let tabs = vec![tab(Some("frontend"), "web", 1), tab(None, "", 0)];
        store.save(&tabs).unwrap();
        assert_eq!(
            store.load().unwrap(),
            vec![tab(None, "", 0), tab(Some("frontend"), "web", 1)]
        );
        let text = std::fs::read_to_string(dir.join("ws/pinned_terminals.json")).unwrap();
        assert!(text.contains("\"version\": 1"), "{text}");
        assert!(!dir.join("ws/pinned_terminals.json.tmp").exists());
    }

    #[test]
    fn corrupted_file_is_an_error() {
        let dir = temp_dir("corrupt");
        let path = dir.join("pinned_terminals.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert!(PinnedStore::at(path.clone()).load().is_err());
        std::fs::write(&path, r#"{"version": 9, "tabs": []}"#).unwrap();
        assert!(PinnedStore::at(path).load().is_err());
    }

    #[test]
    fn encodes_inside_relative_outside_absolute() {
        let root = Path::new("/p/app");
        assert_eq!(encode_cwd(root, Path::new("/p/app/web/src")), "web/src");
        assert_eq!(encode_cwd(root, root), "");
        assert_eq!(encode_cwd(root, Path::new("/tmp/x")), "/tmp/x");
    }

    #[test]
    fn missing_directory_falls_back_to_parent_or_root() {
        let root = temp_dir("resolve");
        std::fs::create_dir_all(root.join("web")).unwrap();
        assert_eq!(resolve_cwd(&root, "web"), root.join("web"));
        assert_eq!(resolve_cwd(&root, "web/gone/deeper"), root.join("web"));
        assert_eq!(resolve_cwd(&root, ""), root);
        assert_eq!(resolve_cwd(&root, "gone"), root);
        let outside = root.join("web");
        assert_eq!(resolve_cwd(&root, &outside.to_string_lossy()), outside);
    }

    #[test]
    fn pinned_go_first_keeping_relative_order() {
        let mut items = vec![(1, false), (2, true), (3, false), (4, true)];
        pinned_first(&mut items, |(_, pinned)| *pinned);
        assert_eq!(items, vec![(2, true), (4, true), (1, false), (3, false)]);
    }
}
