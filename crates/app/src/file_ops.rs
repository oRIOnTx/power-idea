//! Copy / Cut / Paste of project files and folders (project tree menu), and saving
//! pasted images. UI-free: the workspace keeps the clipboard and calls these off the UI thread.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipMode {
    Copy,
    Cut,
}

/// Files and folders waiting to be pasted, relative to the project root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileClipboard {
    pub paths: Vec<String>,
    pub mode: ClipMode,
}

/// `report.pdf` → `report copy.pdf`, then `report copy 2.pdf` (Finder style).
fn copy_name(name: &str, n: usize) -> String {
    let suffix = if n == 1 {
        " copy".to_string()
    } else {
        format!(" copy {n}")
    };
    match name.rsplit_once('.') {
        // Keep the extension; dot-files like `.env` have no extension to keep.
        Some((stem, ext)) if !stem.is_empty() => format!("{stem}{suffix}.{ext}"),
        _ => format!("{name}{suffix}"),
    }
}

/// Where `source` goes when pasted into a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Placement {
    /// The name is free: paste right away.
    Free(PathBuf),
    /// The folder already has `name`; ask for another, starting from `suggested`.
    Taken { name: String, suggested: String },
}

/// Decides where `source` lands when pasted into `dir`.
pub fn placement(source: &Path, dir: &Path, mode: ClipMode) -> Result<Placement> {
    if !source.exists() {
        bail!("{} no longer exists", source.display());
    }
    let name = source
        .file_name()
        .context("invalid path")?
        .to_string_lossy()
        .into_owned();
    if source.is_dir() && dir.starts_with(source) {
        bail!("cannot paste {name} into itself");
    }
    let target = dir.join(&name);
    if !target.exists() {
        return Ok(Placement::Free(target));
    }
    if mode == ClipMode::Cut && target == source {
        bail!("{name} is already in this folder");
    }
    let suggested = (1..usize::MAX)
        .map(|n| copy_name(&name, n))
        .find(|candidate| !dir.join(candidate).exists())
        .context("no free name")?;
    Ok(Placement::Taken { name, suggested })
}

fn copy_recursive(from: &Path, to: &Path) -> Result<()> {
    if from.is_dir() {
        fs::create_dir_all(to).with_context(|| format!("creating {}", to.display()))?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            copy_recursive(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        fs::copy(from, to)
            .map(|_| ())
            .with_context(|| format!("copying {}", from.display()))
    }
}

/// `image.png`, then `image 2.png`, … — the first name not taken in `dir`.
fn free_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    (1..usize::MAX)
        .map(|n| match n {
            1 => dir.join(format!("{stem}.{ext}")),
            n => dir.join(format!("{stem} {n}.{ext}")),
        })
        .find(|candidate| !candidate.exists())
        .unwrap_or_else(|| dir.join(format!("{stem}.{ext}")))
}

/// Saves image bytes from the system clipboard (e.g. a screenshot) as a new file in `dir`.
pub fn save_image(dir: &Path, ext: &str, bytes: &[u8]) -> Result<PathBuf> {
    let target = free_path(dir, "image", ext);
    fs::write(&target, bytes).with_context(|| format!("writing {}", target.display()))?;
    Ok(target)
}

/// Copies or moves `source` to `target`, which must not exist yet.
pub fn paste_to(source: &Path, target: &Path, mode: ClipMode) -> Result<()> {
    if !source.exists() {
        bail!("{} no longer exists", source.display());
    }
    if target.exists() {
        bail!("{} already exists", target.display());
    }
    if source.is_dir() && target.starts_with(source) {
        bail!("cannot paste {} into itself", source.display());
    }
    match mode {
        ClipMode::Copy => copy_recursive(source, target)?,
        // A plain rename fails across volumes; copy and remove then.
        ClipMode::Cut => {
            if fs::rename(source, target).is_err() {
                copy_recursive(source, target)?;
                if source.is_dir() {
                    fs::remove_dir_all(source)?;
                } else {
                    fs::remove_file(source)?;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::{ClipMode, Placement, copy_name, paste_to, placement, save_image};

    #[test]
    fn names_copies_like_finder() {
        assert_eq!(copy_name("report.pdf", 1), "report copy.pdf");
        assert_eq!(copy_name("report.pdf", 2), "report copy 2.pdf");
        assert_eq!(copy_name("models", 1), "models copy");
        assert_eq!(copy_name(".env", 1), ".env copy");
        assert_eq!(copy_name("archive.tar.gz", 1), "archive.tar copy.gz");
    }

    #[test]
    fn copies_and_moves_files_and_folders() {
        let root = std::env::temp_dir().join(format!("power-idea-file-ops-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("app/models")).unwrap();
        fs::create_dir_all(root.join("lib")).unwrap();
        fs::write(root.join("app/models/user.rb"), "class User; end").unwrap();
        let models = root.join("app/models");
        let user = models.join("user.rb");

        // Same folder: the name is taken; the suggestion skips names already used.
        let taken = |n: &str| Placement::Taken {
            name: "user.rb".into(),
            suggested: n.into(),
        };
        assert_eq!(
            placement(&user, &models, ClipMode::Copy).unwrap(),
            taken("user copy.rb")
        );
        paste_to(&user, &models.join("user copy.rb"), ClipMode::Copy).unwrap();
        assert_eq!(
            placement(&user, &models, ClipMode::Copy).unwrap(),
            taken("user copy 2.rb")
        );

        // A folder is copied with its contents.
        let lib = root.join("lib");
        assert_eq!(
            placement(&models, &lib, ClipMode::Copy).unwrap(),
            Placement::Free(lib.join("models"))
        );
        paste_to(&models, &lib.join("models"), ClipMode::Copy).unwrap();
        assert_eq!(
            fs::read_to_string(lib.join("models/user.rb")).unwrap(),
            "class User; end"
        );

        // Cut moves; the source is gone. A taken name in another folder asks for a new one.
        paste_to(
            &models.join("user copy.rb"),
            &lib.join("user copy.rb"),
            ClipMode::Cut,
        )
        .unwrap();
        assert!(lib.join("user copy.rb").exists() && !models.join("user copy.rb").exists());
        assert!(matches!(
            placement(&user, &lib.join("models"), ClipMode::Cut).unwrap(),
            Placement::Taken { .. }
        ));

        // Guards: into itself, cut into the same folder, onto an existing file, a missing source.
        assert!(placement(&root.join("app"), &models, ClipMode::Copy).is_err());
        assert!(paste_to(&root.join("app"), &models.join("app"), ClipMode::Copy).is_err());
        assert!(placement(&user, &models, ClipMode::Cut).is_err());
        assert!(paste_to(&user, &lib.join("models/user.rb"), ClipMode::Copy).is_err());
        assert!(placement(Path::new("/nonexistent/x"), &models, ClipMode::Copy).is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn saves_pasted_images_under_free_names() {
        let dir = std::env::temp_dir().join(format!("power-idea-paste-image-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        assert_eq!(save_image(&dir, "png", b"one").unwrap(), dir.join("image.png"));
        assert_eq!(save_image(&dir, "png", b"two").unwrap(), dir.join("image 2.png"));
        assert_eq!(fs::read(dir.join("image 2.png")).unwrap(), b"two");
        assert!(save_image(&dir.join("missing"), "png", b"x").is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
