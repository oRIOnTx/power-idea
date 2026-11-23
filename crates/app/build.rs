//! Embeds the version computed by `scripts/version.sh` from git tags (see that script):
//! `POWER_IDEA_VERSION` (marketing), `POWER_IDEA_BUILD`, `POWER_IDEA_SHA` and
//! `POWER_IDEA_FULL` ("1.5.0 (412)" for a release, "1.5.0-dev.3+abc1234[.dirty]" otherwise).
//! Never fails the build: without git or bash it falls back to APP_VERSION / APP_BUILD.

use std::process::Command;

#[derive(Debug)]
struct Version {
    marketing: String,
    build: String,
    sha: String,
    full: String,
}

/// Parses the script's `KEY=value` lines (values are `printf %q` quoted).
fn parse(output: &str) -> Option<Version> {
    let get = |key: &str| {
        output.lines().find_map(|line| {
            let value = line.strip_prefix(key)?.strip_prefix('=')?;
            Some(unquote(value))
        })
    };
    Some(Version {
        marketing: get("VERSION")?,
        build: get("BUILD")?,
        sha: get("SHA")?,
        full: get("FULL")?,
    })
}

/// Undoes bash `%q` quoting for the characters version strings contain (`\ `, `\(`, …).
fn unquote(value: &str) -> String {
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.extend(chars.next()),
            '\'' => {}
            c => out.push(c),
        }
    }
    out
}

fn from_script() -> Option<Version> {
    let output = Command::new("bash")
        .arg("../../scripts/version.sh")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse(&String::from_utf8_lossy(&output.stdout))
}

fn fallback() -> Version {
    let version = std::env::var("APP_VERSION").unwrap_or_else(|_| "0.1.0".into());
    let build = std::env::var("APP_BUILD").unwrap_or_else(|_| "0".into());
    Version {
        full: format!("{version} ({build})"),
        marketing: version,
        build,
        sha: std::env::var("GIT_SHA").unwrap_or_else(|_| "unknown".into()),
    }
}

fn main() {
    let version = from_script().unwrap_or_else(fallback);
    println!("cargo:rustc-env=POWER_IDEA_VERSION={}", version.marketing);
    println!("cargo:rustc-env=POWER_IDEA_BUILD={}", version.build);
    println!("cargo:rustc-env=POWER_IDEA_SHA={}", version.sha);
    println!("cargo:rustc-env=POWER_IDEA_FULL={}", version.full);

    for var in ["APP_VERSION", "APP_BUILD", "GIT_SHA"] {
        println!("cargo:rerun-if-env-changed={var}");
    }
    // Re-run on commits, checkouts, new tags and any source edit (for the dirty mark).
    for path in [
        "../../scripts/version.sh",
        "../../.git/HEAD",
        "../../.git/index",
        "../../.git/refs/heads",
        "../../.git/refs/tags",
        "../../.git/packed-refs",
        "..",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
}
