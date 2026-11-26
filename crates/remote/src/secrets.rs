//! Site passwords and key passphrases in the macOS login Keychain, through the
//! Security framework (the secret never appears on a command line or in a file).

use anyhow::{Context as _, Result};
use security_framework::passwords;

const SERVICE: &str = "Power IDEA remote";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Secret {
    Password,
    Passphrase,
}

fn account(site_id: &str, secret: Secret) -> String {
    match secret {
        Secret::Password => format!("{site_id}:password"),
        Secret::Passphrase => format!("{site_id}:passphrase"),
    }
}

pub fn store(site_id: &str, secret: Secret, value: &str) -> Result<()> {
    passwords::set_generic_password(SERVICE, &account(site_id, secret), value.as_bytes())
        .context("could not save to the Keychain")
}

/// The stored value, or `None` when there is none (or the Keychain refused).
pub fn load(site_id: &str, secret: Secret) -> Option<String> {
    let bytes = passwords::get_generic_password(SERVICE, &account(site_id, secret)).ok()?;
    String::from_utf8(bytes).ok()
}

/// Removes a stored value; a missing one is not an error.
pub fn delete(site_id: &str, secret: Secret) {
    passwords::delete_generic_password(SERVICE, &account(site_id, secret)).ok();
}

/// Forgets every secret of a deleted site.
pub fn delete_all(site_id: &str) {
    delete(site_id, Secret::Password);
    delete(site_id, Secret::Passphrase);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accounts_are_per_site_and_kind() {
        assert_eq!(account("box-1", Secret::Password), "box-1:password");
        assert_eq!(account("box-1", Secret::Passphrase), "box-1:passphrase");
    }
}
