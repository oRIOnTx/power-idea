//! Run configurations guessed from project files, for any ecosystem.

use std::fs;
use std::path::Path;

use super::{DebugSpec, RunConfig};

/// Make targets listed at most (large Makefiles have dozens of helpers).
const MAX_MAKE_TARGETS: usize = 15;

fn has(root: &Path, file: &str) -> bool {
    root.join(file).exists()
}

/// JavaScript package runner chosen by the lockfile in use.
fn js_runner(root: &Path) -> &'static str {
    if has(root, "pnpm-lock.yaml") {
        "pnpm run"
    } else if has(root, "yarn.lock") {
        "yarn"
    } else if has(root, "bun.lockb") || has(root, "bun.lock") {
        "bun run"
    } else {
        "npm run"
    }
}

/// `scripts` of package.json, alphabetically.
fn package_scripts(text: &str) -> Vec<String> {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    json.get("scripts")
        .and_then(|s| s.as_object())
        .map_or_else(Vec::new, |scripts| {
            // Sorted explicitly: the map's order depends on serde_json features.
            let mut names: Vec<String> = scripts.keys().cloned().collect();
            names.sort();
            names
        })
}

/// Plain targets of a Makefile (`name:` at line start, not `.PHONY` or `VAR := …`).
pub(crate) fn make_targets(text: &str) -> Vec<String> {
    let mut targets = Vec::new();
    for line in text.lines() {
        let Some((name, rest)) = line.split_once(':') else {
            continue;
        };
        let plain = !name.is_empty()
            && !name.starts_with(['.', '\t', ' ', '#'])
            && name
                .chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '/'))
            && !rest.starts_with('=');
        if plain && !targets.contains(&name.to_string()) {
            targets.push(name.to_string());
        }
    }
    targets.truncate(MAX_MAKE_TARGETS);
    targets
}

/// Configurations that fit the project at `root`.
pub fn detect(root: &Path) -> Vec<RunConfig> {
    let mut configs = Vec::new();
    let mut add = |config: RunConfig| configs.push(config);
    let ruby = || DebugSpec::adapter("ruby");

    // Ruby / Rails: rdbg debugs the command itself.
    if has(root, "Gemfile") {
        if has(root, "bin/rails") {
            add(RunConfig::new("Rails server", "bin/rails server").with_debug(ruby()));
            add(RunConfig::new("Rails console", "bin/rails console").with_debug(ruby()));
            if has(root, "test") {
                add(RunConfig::new("Rails tests", "bin/rails test").with_debug(ruby()));
            }
        }
        if has(root, "spec") {
            add(RunConfig::new("RSpec", "bundle exec rspec").with_debug(ruby()));
        }
    }
    // Rust: lldb-dap debugs the binary that `cargo build` produces.
    if has(root, "Cargo.toml") {
        let run = RunConfig::new("cargo run", "cargo run");
        add(match cargo_binary(root) {
            Some(binary) => run.with_debug(DebugSpec {
                program: Some(format!("target/debug/{binary}")),
                build: Some("cargo build".into()),
                ..DebugSpec::adapter("lldb")
            }),
            None => run,
        });
        add(RunConfig::new("cargo test", "cargo test"));
    }
    // JavaScript / TypeScript
    if let Ok(text) = fs::read_to_string(root.join("package.json")) {
        let runner = js_runner(root);
        for script in package_scripts(&text) {
            add(RunConfig::new(
                format!("{runner} {script}"),
                format!("{runner} {script}"),
            ));
        }
    }
    // Go
    if has(root, "go.mod") {
        add(RunConfig::new("go run", "go run .").with_debug(DebugSpec {
            program: Some(".".into()),
            ..DebugSpec::adapter("go")
        }));
        add(RunConfig::new("go test", "go test ./..."));
    }
    // Python
    if has(root, "manage.py") {
        add(
            RunConfig::new("Django server", "python manage.py runserver").with_debug(DebugSpec {
                program: Some("manage.py".into()),
                args: vec!["runserver".into(), "--noreload".into()],
                ..DebugSpec::adapter("python")
            }),
        );
    }
    if has(root, "pytest.ini")
        || has(root, "conftest.py")
        || (has(root, "pyproject.toml") && has(root, "tests"))
    {
        add(RunConfig::new("pytest", "pytest").with_debug(DebugSpec {
            module: Some("pytest".into()),
            ..DebugSpec::adapter("python")
        }));
    }
    // Elixir
    if has(root, "mix.exs") {
        add(RunConfig::new("mix test", "mix test"));
    }
    // Make
    if let Ok(text) = fs::read_to_string(root.join("Makefile")) {
        for target in make_targets(&text) {
            add(RunConfig::new(format!("make {target}"), format!("make {target}")));
        }
    }
    configs
}

/// Name of the binary `cargo build` writes for a single-package project.
fn cargo_binary(root: &Path) -> Option<String> {
    let text = fs::read_to_string(root.join("Cargo.toml")).ok()?;
    let manifest: toml::Value = toml::from_str(&text).ok()?;
    let bins = manifest.get("bin").and_then(|b| b.as_array());
    let first_bin = bins
        .and_then(|b| b.first())
        .and_then(|b| b.get("name"))
        .and_then(|n| n.as_str());
    first_bin
        .or_else(|| manifest.get("package")?.get("name")?.as_str())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{detect, make_targets};

    #[test]
    fn detects_common_ecosystems() {
        let dir = std::env::temp_dir().join(format!("power-idea-detect-{}", std::process::id()));
        fs::create_dir_all(dir.join("bin")).unwrap();
        fs::create_dir_all(dir.join("spec")).unwrap();
        for (file, text) in [
            ("Gemfile", "source 'https://rubygems.org'"),
            ("bin/rails", "#!/usr/bin/env ruby"),
            (
                "package.json",
                r#"{"scripts": {"dev": "vite", "build": "vite build"}}"#,
            ),
            ("yarn.lock", ""),
            ("go.mod", "module x"),
            (
                "Makefile",
                ".PHONY: test\nVAR := 1\ntest: build\n\tgo test\nbuild:\n\tgo build\n",
            ),
        ] {
            fs::write(dir.join(file), text).unwrap();
        }
        let configs = detect(&dir);
        assert_eq!(
            configs[0].debug.as_ref().map(|d| d.adapter.as_str()),
            Some("ruby")
        );
        assert!(
            configs
                .iter()
                .find(|c| c.name == "go run")
                .unwrap()
                .debug
                .is_some()
        );
        let names: Vec<String> = configs.into_iter().map(|c| c.name).collect();
        assert_eq!(
            names,
            [
                "Rails server",
                "Rails console",
                "RSpec",
                "yarn build",
                "yarn dev",
                "go run",
                "go test",
                "make test",
                "make build"
            ]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn finds_the_cargo_binary() {
        let dir = std::env::temp_dir().join(format!("power-idea-cargo-bin-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("Cargo.toml"), "[package]\nname = \"tool\"\n").unwrap();
        let run = detect(&dir).into_iter().find(|c| c.name == "cargo run").unwrap();
        let debug = run.debug.unwrap();
        assert_eq!(
            (debug.adapter.as_str(), debug.program.as_deref()),
            ("lldb", Some("target/debug/tool"))
        );
        assert_eq!(debug.build.as_deref(), Some("cargo build"));
        // A workspace manifest has no single binary to debug.
        fs::write(dir.join("Cargo.toml"), "[workspace]\nmembers = [\"a\"]\n").unwrap();
        assert!(
            detect(&dir)
                .into_iter()
                .find(|c| c.name == "cargo run")
                .unwrap()
                .debug
                .is_none()
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_make_targets() {
        assert_eq!(
            make_targets(
                "all: a b\n\nlint:\n\t@x\n.DEFAULT_GOAL := all\nX ?= 1\nfoo := bar\n# c: d\n%.o: %.c\n"
            ),
            ["all", "lint"]
        );
    }
}
