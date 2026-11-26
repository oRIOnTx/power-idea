//! Local copies of opened remote files:
//! `~/Library/Caches/power-idea/remote/<site id>/<remote path>`.

use std::path::{Component, Path, PathBuf};

/// The default cache root, or `None` without a cache folder.
pub fn default_root() -> Option<PathBuf> {
    dirs::cache_dir().map(|dir| dir.join("power-idea").join("remote"))
}

/// Where the local copy of `remote_path` on `site_id` lives. Every component is a
/// plain name, so `..` in a server path can never escape the cache folder.
pub fn local_path(root: &Path, site_id: &str, remote_path: &str) -> PathBuf {
    let mut path = root.join(sanitize(site_id));
    for part in remote_path.split('/') {
        let part = sanitize(part);
        if !part.is_empty() {
            path.push(part);
        }
    }
    debug_assert!(path.components().all(|c| !matches!(c, Component::ParentDir)));
    path
}

fn sanitize(part: &str) -> String {
    match part {
        "" | "." => String::new(),
        ".." => "_..".into(),
        other => other.replace(['\\', '\0'], "_"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_remote_paths_inside_the_root() {
        let root = Path::new("/cache");
        assert_eq!(
            local_path(root, "box", "/etc/nginx/nginx.conf"),
            Path::new("/cache/box/etc/nginx/nginx.conf")
        );
        assert_eq!(
            local_path(root, "box", "/a/../../b"),
            Path::new("/cache/box/a/_../_../b")
        );
        assert_eq!(local_path(root, "..", "/x"), Path::new("/cache/_../x"));
    }
}
