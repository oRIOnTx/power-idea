//! POSIX paths on the server: joining, parents, names and normalising `.` / `..`.
//! Remote paths are always `/`-separated strings, whatever the local OS.

/// `dir` + `name`, with exactly one `/` between them.
pub fn join(dir: &str, name: &str) -> String {
    let name = name.trim_start_matches('/');
    if dir.is_empty() {
        return name.to_string();
    }
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// The parent directory; `/` is its own parent.
pub fn parent(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) => "/".into(),
        Some(ix) => trimmed[..ix].to_string(),
        None if path.starts_with('/') => "/".into(),
        None => String::new(),
    }
}

/// The last component (`""` for `/`).
pub fn file_name(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    trimmed.rsplit('/').next().unwrap_or(trimmed)
}

/// Resolves `.`, `..` and repeated slashes; a relative `path` is taken from `base`.
/// `..` never climbs above `/`.
pub fn normalize(base: &str, path: &str) -> String {
    let path = path.trim();
    let full = if path.starts_with('/') {
        path.to_string()
    } else if path.is_empty() {
        base.to_string()
    } else {
        join(base, path)
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in full.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    format!("/{}", parts.join("/"))
}

/// Whether `path` is `dir` or inside it (both normalised).
pub fn is_within(dir: &str, path: &str) -> bool {
    let dir = dir.trim_end_matches('/');
    dir.is_empty() || path == dir || path.starts_with(&format!("{dir}/"))
}

/// Every directory from `/` down to `path`, for a breadcrumb: `[("/", "/"), ("etc", "/etc"), …]`.
pub fn breadcrumbs(path: &str) -> Vec<(String, String)> {
    let mut crumbs = vec![("/".to_string(), "/".to_string())];
    let mut current = String::new();
    for part in path.split('/').filter(|p| !p.is_empty()) {
        current = format!("{current}/{part}");
        crumbs.push((part.to_string(), current.clone()));
    }
    crumbs
}

/// A name for a new file or folder: one path component, nothing that escapes the folder.
pub fn validate_name(name: &str) -> Result<&str, &'static str> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Name is empty");
    }
    if name.contains('/') || name == "." || name == ".." || name.contains('\0') {
        return Err("Name cannot contain '/' or be '.' or '..'");
    }
    Ok(name)
}

/// Quotes `text` for a POSIX shell (single quotes, `'` escaped).
pub fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins() {
        assert_eq!(join("/var", "log"), "/var/log");
        assert_eq!(join("/", "etc"), "/etc");
        assert_eq!(join("/var/", "/log"), "/var/log");
        assert_eq!(join("", "a"), "a");
    }

    #[test]
    fn parents_and_names() {
        assert_eq!(parent("/var/log/syslog"), "/var/log");
        assert_eq!(parent("/var"), "/");
        assert_eq!(parent("/"), "/");
        assert_eq!(parent("/var/log/"), "/var");
        assert_eq!(file_name("/var/log/syslog"), "syslog");
        assert_eq!(file_name("/var/log/"), "log");
        assert_eq!(file_name("/"), "");
    }

    #[test]
    fn normalizes() {
        assert_eq!(
            normalize("/home/me", "app/../logs/./x.log"),
            "/home/me/logs/x.log"
        );
        assert_eq!(normalize("/home/me", "/etc//nginx/"), "/etc/nginx");
        assert_eq!(normalize("/", "../../.."), "/");
        assert_eq!(normalize("/srv", ""), "/srv");
        assert_eq!(normalize("/srv", "~"), "/srv/~");
    }

    #[test]
    fn containment() {
        assert!(is_within("/srv/app", "/srv/app/x"));
        assert!(is_within("/srv/app", "/srv/app"));
        assert!(!is_within("/srv/app", "/srv/application"));
        assert!(is_within("/", "/anything"));
    }

    #[test]
    fn crumbs() {
        let crumbs = breadcrumbs("/var/log");
        assert_eq!(
            crumbs,
            vec![
                ("/".into(), "/".into()),
                ("var".into(), "/var".into()),
                ("log".into(), "/var/log".into())
            ]
        );
    }

    #[test]
    fn names() {
        assert_eq!(validate_name(" a.txt "), Ok("a.txt"));
        assert!(validate_name("a/b").is_err());
        assert!(validate_name("..").is_err());
        assert!(validate_name("").is_err());
    }

    #[test]
    fn quotes_for_shell() {
        assert_eq!(shell_quote("/srv/it's"), r"'/srv/it'\''s'");
    }
}
