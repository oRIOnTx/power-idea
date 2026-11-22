//! Embeds the build number (git commit count) and the commit hash, so every
//! change gets a distinct, visible app version without bumping it by hand.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn main() {
    let build = git(&["rev-list", "--count", "HEAD"]).unwrap_or_else(|| "0".into());
    let commit = git(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown".into());
    // `+` marks a build with uncommitted changes in tracked files.
    let dirty = git(&["status", "--porcelain", "--untracked-files=no"]).is_some_and(|s| !s.is_empty());
    println!("cargo:rustc-env=POWER_IDEA_BUILD={build}");
    println!(
        "cargo:rustc-env=POWER_IDEA_COMMIT={commit}{}",
        if dirty { "+" } else { "" }
    );
    // Re-run on commits/checkouts and on any source edit (for the dirty mark).
    for path in [
        "../../.git/HEAD",
        "../../.git/index",
        "../../.git/refs/heads",
        "..",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
}
