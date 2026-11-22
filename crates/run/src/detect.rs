//! Run configurations guessed from project files, for any ecosystem.

use std::fs;
use std::path::Path;

use super::RunConfig;

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
    let mut add = |name: String, command: String| configs.push(RunConfig::new(name, command));

    // Ruby / Rails
    if has(root, "Gemfile") {
        if has(root, "bin/rails") {
            add("Rails server".into(), "bin/rails server".into());
            add("Rails console".into(), "bin/rails console".into());
            if has(root, "test") {
                add("Rails tests".into(), "bin/rails test".into());
            }
        }
        if has(root, "spec") {
            add("RSpec".into(), "bundle exec rspec".into());
        }
    }
    // Rust
    if has(root, "Cargo.toml") {
        add("cargo run".into(), "cargo run".into());
        add("cargo test".into(), "cargo test".into());
    }
    // JavaScript / TypeScript
    if let Ok(text) = fs::read_to_string(root.join("package.json")) {
        let runner = js_runner(root);
        for script in package_scripts(&text) {
            add(format!("{runner} {script}"), format!("{runner} {script}"));
        }
    }
    // Go
    if has(root, "go.mod") {
        add("go run".into(), "go run .".into());
        add("go test".into(), "go test ./...".into());
    }
    // Python
    if has(root, "manage.py") {
        add("Django server".into(), "python manage.py runserver".into());
    }
    if has(root, "pytest.ini")
        || has(root, "conftest.py")
        || (has(root, "pyproject.toml") && has(root, "tests"))
    {
        add("pytest".into(), "pytest".into());
    }
    // Elixir
    if has(root, "mix.exs") {
        add("mix test".into(), "mix test".into());
    }
    // Make
    if let Ok(text) = fs::read_to_string(root.join("Makefile")) {
        for target in make_targets(&text) {
            add(format!("make {target}"), format!("make {target}"));
        }
    }
    configs
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
        let names: Vec<String> = detect(&dir).into_iter().map(|c| c.name).collect();
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
    fn parses_make_targets() {
        assert_eq!(
            make_targets(
                "all: a b\n\nlint:\n\t@x\n.DEFAULT_GOAL := all\nX ?= 1\nfoo := bar\n# c: d\n%.o: %.c\n"
            ),
            ["all", "lint"]
        );
    }
}
