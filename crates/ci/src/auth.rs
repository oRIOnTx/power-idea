//! Where the GitHub token comes from: the `gh` CLI first, then a personal access token
//! kept in the macOS login Keychain. The token is never logged, printed or put in argv.

use std::fmt;
use std::io::Write as _;
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, bail};

/// A secret that cannot leak through `Debug` or `Display`.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl Token {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The raw value, for the Authorization header only.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(***)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenSource {
    GhCli,
    Keychain,
}

impl TokenSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::GhCli => "GitHub CLI",
            Self::Keychain => "personal access token",
        }
    }
}

/// The first token found: `gh auth token`, then the Keychain. Blocks: run it off the UI thread.
pub fn resolve_token() -> Option<(Token, TokenSource)> {
    if let Some(token) = gh_token() {
        return Some((token, TokenSource::GhCli));
    }
    load_pat().map(|token| (token, TokenSource::Keychain))
}

/// `gh` is installed (on the PATH the IDE got from the login shell).
pub fn gh_installed() -> bool {
    Command::new("gh")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn gh_token() -> Option<Token> {
    let output = Command::new("gh")
        .args(["auth", "token", "--hostname", "github.com"])
        .env("GH_PROMPT_DISABLED", "1")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let token = String::from_utf8(output.stdout).ok()?;
    let token = token.trim();
    is_valid_token(token).then(|| Token::new(token))
}

const SERVICE: &str = "Power IDEA GitHub";
const ACCOUNT: &str = "github.com";

/// GitHub tokens (`ghp_…`, `github_pat_…`, `gho_…`) are ASCII letters, digits and `_`.
pub fn is_valid_token(token: &str) -> bool {
    (20..=255).contains(&token.len()) && token.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

pub fn load_pat() -> Option<Token> {
    let output = Command::new("security")
        .args(["find-generic-password", "-s", SERVICE, "-a", ACCOUNT, "-w"])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let token = String::from_utf8(output.stdout).ok()?;
    let token = token.trim();
    is_valid_token(token).then(|| Token::new(token))
}

/// Saves a personal access token. `security -i` reads the command from stdin, so the token
/// never appears in the process list; validation keeps it from breaking out of the quotes.
pub fn store_pat(token: &str) -> Result<()> {
    let token = token.trim();
    if !is_valid_token(token) {
        bail!("this does not look like a GitHub token (letters, digits and _ only)");
    }
    let mut child = Command::new("security")
        .arg("-i")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("running security")?;
    let command = format!("add-generic-password -U -s \"{SERVICE}\" -a \"{ACCOUNT}\" -w \"{token}\"\n");
    child
        .stdin
        .take()
        .context("security has no stdin")?
        .write_all(command.as_bytes())
        .context("writing to security")?;
    let status = child.wait().context("waiting for security")?;
    if !status.success() || load_pat().is_none_or(|saved| saved.expose() != token) {
        bail!("could not save the token to the Keychain");
    }
    Ok(())
}

pub fn delete_pat() {
    let _ = Command::new("security")
        .args(["delete-generic-password", "-s", SERVICE, "-a", ACCOUNT])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_debug_is_redacted() {
        let token = Token::new("ghp_abcdefghijklmnopqrstuvwxyz");
        assert_eq!(format!("{token:?}"), "Token(***)");
        assert_eq!(format!("{:?}", Some(token)), "Some(Token(***))");
    }

    #[test]
    fn validates_token_shape() {
        assert!(is_valid_token("ghp_abcdefghijklmnopqrstuvwxyz0123456789"));
        assert!(is_valid_token("github_pat_11ABCDEFG0abcdefghijkl_mnopqrstuvwxyz"));
        assert!(!is_valid_token("short"));
        assert!(!is_valid_token("ghp_abcdefghijklmnop\" -w x; delete"));
        assert!(!is_valid_token("ghp_abcdefghijklmnopqrstuvwxyz\n"));
    }
}
