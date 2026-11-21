//! Data source passwords in the macOS login Keychain (via `security`), so
//! they never land in config files.

use std::process::Command;

use anyhow::{Result, bail};

const SERVICE: &str = "Power IDEA database";

pub fn store_password(connection_id: &str, password: &str) -> Result<()> {
    let output = Command::new("security")
        .args([
            "add-generic-password",
            "-U",
            "-s",
            SERVICE,
            "-a",
            connection_id,
            "-w",
            password,
        ])
        .output()?;
    if !output.status.success() {
        bail!("could not save the password to the Keychain");
    }
    Ok(())
}

/// The stored password, or `None` if there is none.
pub fn load_password(connection_id: &str) -> Option<String> {
    let output = Command::new("security")
        .args(["find-generic-password", "-s", SERVICE, "-a", connection_id, "-w"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let password = String::from_utf8(output.stdout).ok()?;
    Some(password.trim_end_matches('\n').to_string())
}

pub fn delete_password(connection_id: &str) {
    let _ = Command::new("security")
        .args(["delete-generic-password", "-s", SERVICE, "-a", connection_id])
        .output();
}
