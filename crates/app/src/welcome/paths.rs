//! Path rules of the Welcome screen: where new projects go and how paths are shown.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

/// Folder suggested for a new project: next to the most recent one, else `~/Projects`.
pub fn default_location(recent: &[PathBuf], home: &Path) -> PathBuf {
    recent
        .first()
        .and_then(|p| p.parent())
        .map_or_else(|| home.join("Projects"), Path::to_path_buf)
}

/// `/Users/me/x` → `~/x` for display.
pub fn tilde(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// `~/x` typed by the user → `/Users/me/x`.
pub fn expand_tilde(text: &str, home: &Path) -> PathBuf {
    match text.trim() {
        "~" => home.to_path_buf(),
        text => match text.strip_prefix("~/") {
            Some(rest) => home.join(rest),
            None => PathBuf::from(text),
        },
    }
}

/// Validates the new project's folder: `location/name`, which must not exist
/// or be an empty folder.
pub fn new_project_dir(location: &Path, name: &str) -> Result<PathBuf> {
    let name = name.trim();
    if name.is_empty() {
        bail!("Enter a project name");
    }
    if name == "." || name == ".." || name.contains('/') {
        bail!("The name cannot contain “/” or be “.” or “..”");
    }
    if !location.is_absolute() {
        bail!("Choose an absolute location");
    }
    let dir = location.join(name);
    if dir.is_file() {
        bail!("{} is a file", dir.display());
    }
    if dir.is_dir() && dir.read_dir()?.next().is_some() {
        bail!("{} already exists and is not empty", dir.display());
    }
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{default_location, expand_tilde, new_project_dir, tilde};

    #[test]
    fn suggests_location_and_shows_tilde() {
        let home = Path::new("/Users/me");
        assert_eq!(default_location(&[], home), PathBuf::from("/Users/me/Projects"));
        let recent = [PathBuf::from("/Users/me/dev/app")];
        assert_eq!(default_location(&recent, home), PathBuf::from("/Users/me/dev"));
        assert_eq!(tilde(Path::new("/Users/me/dev/app"), home), "~/dev/app");
        assert_eq!(tilde(Path::new("/opt/x"), home), "/opt/x");
        assert_eq!(expand_tilde(" ~/dev ", home), PathBuf::from("/Users/me/dev"));
        assert_eq!(expand_tilde("/tmp", home), PathBuf::from("/tmp"));
    }

    #[test]
    fn validates_new_project_folder() {
        let base = std::env::temp_dir().join(format!("power-idea-welcome-{}", std::process::id()));
        fs::create_dir_all(base.join("taken")).unwrap();
        fs::write(base.join("taken/a.txt"), "x").unwrap();
        fs::create_dir_all(base.join("empty")).unwrap();

        assert_eq!(new_project_dir(&base, " app ").unwrap(), base.join("app"));
        assert_eq!(new_project_dir(&base, "empty").unwrap(), base.join("empty"));
        assert!(new_project_dir(&base, "taken").is_err());
        assert!(new_project_dir(&base, "taken/a.txt").is_err());
        assert!(new_project_dir(&base, "").is_err());
        assert!(new_project_dir(&base, "..").is_err());
        assert!(new_project_dir(Path::new("rel"), "app").is_err());
        fs::remove_dir_all(&base).unwrap();
    }
}
