//! Install mode: rules as a managed block in `~/.claude/CLAUDE.md` and skills as managed
//! `~/.claude/skills/ide-<name>/` copies, recorded in a manifest so "Uninstall" removes exactly
//! what the IDE added. Existing files are backed up before their first change. The same block
//! editing serves "Export to project" (a project's CLAUDE.md).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};

use crate::skills;

pub const BLOCK_START: &str = "<!-- ide:managed:start -->";
pub const BLOCK_END: &str = "<!-- ide:managed:end -->";
/// Prefix of the skill folders the IDE manages in `~/.claude/skills`.
pub const SKILL_PREFIX: &str = "ide-";
/// Inside every managed skill copy, so a folder the user made is never taken for ours.
const SKILL_MARKER: &str = ".power-idea-managed";
const MANIFEST_FILE: &str = "install-manifest.json";

/// `text` with `content` in the managed block: replaced in place when the block exists, else
/// appended. Returns the new text and the separator inserted before an appended block.
pub fn upsert_block(text: &str, content: &str) -> (String, String) {
    let block = format!(
        "{BLOCK_START}\n<!-- Managed by Power IDEA (Settings → Claude Code). Edits inside this block are overwritten. -->\n{}\n{BLOCK_END}\n",
        content.trim_end()
    );
    if let Some((before, after)) = find_block(text) {
        return (format!("{before}{block}{after}"), String::new());
    }
    let separator = match text {
        "" => "",
        t if t.ends_with("\n\n") => "",
        t if t.ends_with('\n') => "\n",
        _ => "\n\n",
    };
    (format!("{text}{separator}{block}"), separator.to_string())
}

/// `text` without the managed block (and without `separator`, the text `upsert_block` put
/// before it). `None` when there is no block.
pub fn remove_block(text: &str, separator: &str) -> Option<String> {
    let (before, after) = find_block(text)?;
    let before = if after.is_empty() {
        before.strip_suffix(separator).unwrap_or(before)
    } else {
        before
    };
    Some(format!("{before}{after}"))
}

/// The text before the block's start marker and after its end marker line.
fn find_block(text: &str) -> Option<(&str, &str)> {
    let start = text.find(BLOCK_START)?;
    let end_marker = start + text.get(start..)?.find(BLOCK_END)?;
    let mut end = end_marker + BLOCK_END.len();
    if text.get(end..)?.starts_with('\n') {
        end += 1;
    }
    Some((text.get(..start)?, text.get(end..)?))
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// The CLAUDE.md block, when installed.
    pub claude_md: Option<FileRecord>,
    /// Managed skill folder names (`ide-<name>`) in `~/.claude/skills`.
    pub skills: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRecord {
    /// The file did not exist before the IDE created it.
    pub created: bool,
    /// Text inserted before the block, removed with it.
    pub separator: String,
}

/// Installs into `claude_home` (`~/.claude`), keeping the manifest and backups in `state_dir`
/// (the IDE's Claude folder).
#[derive(Debug, Clone)]
pub struct Installer {
    pub claude_home: PathBuf,
    pub state_dir: PathBuf,
}

/// What an install or uninstall changed, for the notification.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub changed: Vec<String>,
    /// Things left alone on purpose (e.g. a skill folder the IDE does not own).
    pub skipped: Vec<String>,
}

impl Installer {
    pub fn new(claude_home: impl Into<PathBuf>, state_dir: impl Into<PathBuf>) -> Self {
        Self {
            claude_home: claude_home.into(),
            state_dir: state_dir.into(),
        }
    }

    fn claude_md(&self) -> PathBuf {
        self.claude_home.join("CLAUDE.md")
    }

    fn skills_dir(&self) -> PathBuf {
        self.claude_home.join("skills")
    }

    pub fn manifest(&self) -> Manifest {
        fs::read_to_string(self.state_dir.join(MANIFEST_FILE))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    fn save_manifest(&self, manifest: &Manifest) -> Result<()> {
        fs::create_dir_all(&self.state_dir)?;
        let path = self.state_dir.join(MANIFEST_FILE);
        if *manifest == Manifest::default() {
            if path.exists() {
                fs::remove_file(&path)?;
            }
            return Ok(());
        }
        fs::write(&path, serde_json::to_string_pretty(manifest)?).context("writing the install manifest")
    }

    /// Brings `~/.claude` in line: the rules block (`None` removes it) and exactly `skills`
    /// as managed copies. Running it twice changes nothing the second time.
    pub fn install(&self, rules_text: Option<&str>, skill_dirs: &[PathBuf]) -> Result<Report> {
        let mut manifest = self.manifest();
        let mut report = Report::default();
        self.sync_claude_md(rules_text, &mut manifest, &mut report)?;
        self.sync_skills(skill_dirs, &mut manifest, &mut report)?;
        self.save_manifest(&manifest)?;
        Ok(report)
    }

    /// Removes everything the manifest lists, and nothing else.
    pub fn uninstall(&self) -> Result<Report> {
        self.install(None, &[])
    }

    fn sync_claude_md(
        &self,
        rules_text: Option<&str>,
        manifest: &mut Manifest,
        report: &mut Report,
    ) -> Result<()> {
        let path = self.claude_md();
        let current = match fs::read_to_string(&path) {
            Ok(text) => Some(text),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
            Err(err) => return Err(err).with_context(|| format!("reading {}", path.display())),
        };
        match (rules_text, manifest.claude_md.clone()) {
            (Some(rules), record) => {
                let text = current.clone().unwrap_or_default();
                let (updated, separator) = upsert_block(&text, rules);
                if current.as_deref() == Some(updated.as_str()) {
                    return Ok(());
                }
                if record.is_none() && current.is_some() {
                    self.backup(&path)?;
                }
                fs::create_dir_all(&self.claude_home)?;
                fs::write(&path, &updated).with_context(|| format!("writing {}", path.display()))?;
                manifest.claude_md = Some(record.unwrap_or(FileRecord {
                    created: current.is_none(),
                    separator,
                }));
                report.changed.push(format!("rules block in {}", path.display()));
            }
            (None, Some(record)) => {
                manifest.claude_md = None;
                let Some(text) = current else { return Ok(()) };
                let Some(restored) = remove_block(&text, &record.separator) else {
                    return Ok(());
                };
                if record.created && restored.is_empty() {
                    fs::remove_file(&path)?;
                } else {
                    fs::write(&path, restored)?;
                }
                report
                    .changed
                    .push(format!("removed the rules block from {}", path.display()));
            }
            (None, None) => {}
        }
        Ok(())
    }

    fn sync_skills(
        &self,
        skill_dirs: &[PathBuf],
        manifest: &mut Manifest,
        report: &mut Report,
    ) -> Result<()> {
        let root = self.skills_dir();
        let wanted: Vec<(String, &PathBuf)> = skill_dirs
            .iter()
            .filter_map(|dir| {
                let name = dir.file_name()?.to_string_lossy();
                Some((format!("{SKILL_PREFIX}{name}"), dir))
            })
            .collect();

        // Drop managed copies that are no longer wanted (or everything, on uninstall).
        let mut kept = Vec::new();
        for name in std::mem::take(&mut manifest.skills) {
            let dir = root.join(&name);
            if wanted.iter().any(|(w, _)| *w == name) {
                kept.push(name);
            } else if dir.join(SKILL_MARKER).exists() {
                fs::remove_dir_all(&dir).with_context(|| format!("removing {}", dir.display()))?;
                report.changed.push(format!("removed skill {name}"));
            }
        }
        manifest.skills = kept;

        for (name, source) in wanted {
            let target = root.join(&name);
            if target.exists() && !target.join(SKILL_MARKER).exists() {
                report.skipped.push(format!(
                    "{} exists and is not managed by Power IDEA",
                    target.display()
                ));
                continue;
            }
            let fresh = !target.exists();
            if !fresh && same_tree(source, &target)? {
                continue;
            }
            if target.exists() {
                fs::remove_dir_all(&target)?;
            }
            skills::copy_dir(source, &target)?;
            fs::write(
                target.join(SKILL_MARKER),
                "Managed by Power IDEA; removed by Uninstall.\n",
            )?;
            if !manifest.skills.contains(&name) {
                manifest.skills.push(name.clone());
            }
            report.changed.push(format!(
                "{} skill {name}",
                if fresh { "installed" } else { "updated" }
            ));
        }
        manifest.skills.sort();
        Ok(())
    }

    /// Copies `path` to `<state>/backups/<unix time>-<name>` before its first modification.
    fn backup(&self, path: &Path) -> Result<PathBuf> {
        let dir = self.state_dir.join("backups");
        fs::create_dir_all(&dir)?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let Some(name) = path.file_name() else {
            bail!("cannot back up {}", path.display());
        };
        let target = dir.join(format!("{stamp}-{}", name.to_string_lossy()));
        fs::copy(path, &target).with_context(|| format!("backing up {}", path.display()))?;
        Ok(target)
    }
}

/// Whether the managed copy `target` holds the same files as `source` (ignoring the marker).
fn same_tree(source: &Path, target: &Path) -> Result<bool> {
    let list = |dir: &Path| -> Result<Vec<(PathBuf, Vec<u8>)>> {
        let mut files = Vec::new();
        collect(dir, dir, &mut files)?;
        files.retain(|(rel, _)| rel != Path::new(SKILL_MARKER));
        files.sort();
        Ok(files)
    };
    Ok(list(source)? == list(target)?)
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect(root, &path, out)?;
        } else {
            let rel = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
            out.push((rel, fs::read(&path)?));
        }
    }
    Ok(())
}

/// "Export to project": writes `content` into the managed block of `<project>/CLAUDE.md`.
/// Idempotent; the only way the IDE ever writes into a repository.
pub fn export_to_project(project_root: &Path, content: &str) -> Result<bool> {
    let path = project_root.join("CLAUDE.md");
    let current = fs::read_to_string(&path).unwrap_or_default();
    let (updated, _) = upsert_block(&current, content);
    if updated == current {
        return Ok(false);
    }
    fs::write(&path, updated).with_context(|| format!("writing {}", path.display()))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pi-install-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn block_editing_is_idempotent_and_reversible() {
        for original in ["", "# Mine\n", "# Mine\n\n", "# Mine, no newline"] {
            let (once, separator) = upsert_block(original, "Rules v1");
            let (twice, _) = upsert_block(&once, "Rules v1");
            assert_eq!(once, twice, "upsert twice = once for {original:?}");
            let (updated, _) = upsert_block(&once, "Rules v2");
            assert!(updated.contains("Rules v2") && !updated.contains("Rules v1"));
            assert_eq!(
                remove_block(&updated, &separator).as_deref(),
                Some(original),
                "for {original:?}"
            );
        }
        assert_eq!(remove_block("no block", ""), None);
    }

    #[test]
    fn keeps_user_text_around_the_block() {
        let (installed, separator) = upsert_block("# Top\n", "Rules");
        let edited = format!("{installed}\n# Added later by the user\n");
        let removed = remove_block(&edited, &separator).unwrap();
        assert_eq!(removed, "# Top\n\n\n# Added later by the user\n");
        assert!(!removed.contains(BLOCK_START));
    }

    #[test]
    fn install_uninstall_round_trip_is_byte_identical() {
        let root = temp("roundtrip");
        let (home, state) = (root.join(".claude"), root.join("ide"));
        fs::create_dir_all(home.join("skills/mine")).unwrap();
        let original = "# My global rules\n\nBe brief.";
        fs::write(home.join("CLAUDE.md"), original).unwrap();
        fs::write(home.join("skills/mine/SKILL.md"), "user skill").unwrap();

        let library = root.join("library");
        fs::create_dir_all(library.join("review")).unwrap();
        fs::write(
            library.join("review/SKILL.md"),
            "---\ndescription: R\n---\nReview.",
        )
        .unwrap();
        let installer = Installer::new(&home, &state);

        let report = installer
            .install(Some("## Style\nBe nice."), &[library.join("review")])
            .unwrap();
        assert_eq!(report.changed.len(), 2, "{report:?}");
        let claude_md = fs::read_to_string(home.join("CLAUDE.md")).unwrap();
        assert!(claude_md.starts_with(original) && claude_md.contains("Be nice."));
        assert!(home.join("skills/ide-review/SKILL.md").is_file());
        assert_eq!(
            fs::read_dir(state.join("backups")).unwrap().count(),
            1,
            "backed up before the change"
        );

        let again = installer
            .install(Some("## Style\nBe nice."), &[library.join("review")])
            .unwrap();
        assert!(again.changed.is_empty(), "idempotent: {again:?}");

        installer.uninstall().unwrap();
        assert_eq!(fs::read_to_string(home.join("CLAUDE.md")).unwrap(), original);
        assert!(!home.join("skills/ide-review").exists());
        assert_eq!(
            fs::read_to_string(home.join("skills/mine/SKILL.md")).unwrap(),
            "user skill"
        );
        assert_eq!(installer.manifest(), Manifest::default());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn creates_and_removes_a_missing_claude_md() {
        let root = temp("created");
        let installer = Installer::new(root.join(".claude"), root.join("ide"));
        installer.install(Some("Rules"), &[]).unwrap();
        assert!(root.join(".claude/CLAUDE.md").is_file());
        installer.uninstall().unwrap();
        assert!(
            !root.join(".claude/CLAUDE.md").exists(),
            "a file the IDE created is removed again"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn never_touches_foreign_skill_folders() {
        let root = temp("foreign");
        let home = root.join(".claude");
        fs::create_dir_all(home.join("skills/ide-review")).unwrap();
        fs::write(home.join("skills/ide-review/SKILL.md"), "mine").unwrap();
        let library = root.join("library/review");
        fs::create_dir_all(&library).unwrap();
        fs::write(library.join("SKILL.md"), "ide").unwrap();
        let installer = Installer::new(&home, root.join("ide"));
        let report = installer.install(None, &[library]).unwrap();
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(
            fs::read_to_string(home.join("skills/ide-review/SKILL.md")).unwrap(),
            "mine"
        );
        installer.uninstall().unwrap();
        assert!(home.join("skills/ide-review/SKILL.md").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn exports_to_a_project_idempotently() {
        let root = temp("export");
        fs::write(root.join("CLAUDE.md"), "# Project\n").unwrap();
        assert!(export_to_project(&root, "Rules").unwrap());
        assert!(
            !export_to_project(&root, "Rules").unwrap(),
            "second export changes nothing"
        );
        let text = fs::read_to_string(root.join("CLAUDE.md")).unwrap();
        assert!(text.starts_with("# Project\n\n<!-- ide:managed:start -->"));
        assert_eq!(text.matches(BLOCK_START).count(), 1);
        let _ = fs::remove_dir_all(&root);
    }
}
