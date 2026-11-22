//! Ruby version fallback for servers started through rbenv shims: when the project's Ruby
//! (`.ruby-version`) lacks the server gem or isn't installed, run on the global Ruby or
//! another installed one that has it.

use std::path::{Path, PathBuf};

/// The shim's failure, parsed from the startup error.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum RbenvFailure {
    /// `rbenv: ruby-lsp: command not found`: the gem is missing for the project's Ruby.
    MissingCommand(String),
    /// `rbenv: version '3.2.0' is not installed`.
    MissingVersion(String),
}

pub(super) fn parse_failure(reason: &str) -> Option<RbenvFailure> {
    let rest = &reason[reason.find("rbenv: ")? + "rbenv: ".len()..];
    if let Some(end) = rest.find(": command not found") {
        return Some(RbenvFailure::MissingCommand(rest[..end].trim().to_string()));
    }
    let version = rest.strip_prefix("version ")?;
    let end = version.find(" is not installed")?;
    let quoted = version[..end].trim_matches(|c| matches!(c, '`' | '\'' | '"' | '‘' | '’'));
    Some(RbenvFailure::MissingVersion(quoted.to_string()))
}

/// `RBENV_VERSION` to retry with, and that version, when another installed Ruby has `command`.
pub(super) fn fallback_env(reason: &str, command: &str) -> Option<(Vec<(String, String)>, String)> {
    parse_failure(reason)?;
    let root = rbenv_root()?;
    let name = Path::new(command).file_name()?.to_str()?;
    let installed: Vec<String> = std::fs::read_dir(root.join("versions"))
        .ok()?
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .collect();
    let global = std::fs::read_to_string(root.join("version")).ok();
    let has_command = |version: &str| {
        root.join("versions")
            .join(version)
            .join("bin")
            .join(name)
            .is_file()
    };
    let version = candidates(&installed, global.as_deref().map(str::trim), has_command)
        .into_iter()
        .next()?;
    Some((vec![("RBENV_VERSION".into(), version.clone())], version))
}

fn rbenv_root() -> Option<PathBuf> {
    std::env::var_os("RBENV_ROOT")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".rbenv")))
}

/// Versions that have the command, best first: the global one, then the newest.
fn candidates(installed: &[String], global: Option<&str>, has_command: impl Fn(&str) -> bool) -> Vec<String> {
    let mut versions: Vec<&String> = installed.iter().filter(|v| has_command(v)).collect();
    versions.sort_by_key(|v| std::cmp::Reverse(version_key(v)));
    if let Some(ix) = versions.iter().position(|v| Some(v.as_str()) == global) {
        let global = versions.remove(ix);
        versions.insert(0, global);
    }
    versions.into_iter().cloned().collect()
}

fn version_key(version: &str) -> Vec<u64> {
    version
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|part| part.parse().ok())
        .collect()
}

/// Terminal command that installs what is missing for the project's Ruby.
pub(super) fn fix_command(reason: &str) -> Option<String> {
    Some(match parse_failure(reason)? {
        RbenvFailure::MissingCommand(command) => format!("gem install {command}"),
        // Without arguments `rbenv install` installs the version from `.ruby-version`.
        RbenvFailure::MissingVersion(_) => "rbenv install --skip-existing && gem install ruby-lsp".into(),
    })
}

/// What is wrong, in words, for notifications.
pub(super) fn describe(reason: &str) -> Option<String> {
    Some(match parse_failure(reason)? {
        RbenvFailure::MissingCommand(command) => {
            format!("{command} is not installed for the project's Ruby version (.ruby-version).")
        }
        RbenvFailure::MissingVersion(version) => {
            format!("The project's Ruby {version} (.ruby-version) is not installed.")
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{RbenvFailure, candidates, fix_command, parse_failure};

    #[test]
    fn parses_rbenv_failures() {
        assert_eq!(
            parse_failure("ruby-lsp exited during startup: rbenv: ruby-lsp: command not found"),
            Some(RbenvFailure::MissingCommand("ruby-lsp".into()))
        );
        assert_eq!(
            parse_failure("exited: rbenv: version `3.2.0' is not installed (set by /p/.ruby-version)"),
            Some(RbenvFailure::MissingVersion("3.2.0".into()))
        );
        assert_eq!(parse_failure("Could not find pg-1.5.9"), None);
        assert_eq!(
            fix_command("rbenv: ruby-lsp: command not found").unwrap(),
            "gem install ruby-lsp"
        );
    }

    #[test]
    fn prefers_global_then_newest_ruby_with_the_command() {
        let installed = ["3.2.11", "3.3.13", "3.4.7", "3.10.1"].map(String::from);
        let all = |_: &str| true;
        assert_eq!(candidates(&installed, Some("3.3.13"), all)[0], "3.3.13");
        assert_eq!(
            candidates(&installed, None, all),
            ["3.10.1", "3.4.7", "3.3.13", "3.2.11"]
        );
        // The global Ruby lacks the gem: the newest one that has it wins.
        let only_347 = |v: &str| v == "3.4.7";
        assert_eq!(candidates(&installed, Some("3.3.13"), only_347), ["3.4.7"]);
    }
}
