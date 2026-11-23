//! Code quality: which linters a project uses, how to run them, how to count their errors
//! and warnings, and the score. Pure apart from `detect` reading project files.

use std::fs;
use std::path::Path;

use crate::gauges::QualitySample;
use crate::settings::QualitySettings;

/// How a tool's output is counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parser {
    /// `path:line:col: error|warning…` (cargo clippy `--message-format=short`, rustc style).
    Rustc,
    /// `Diff in …` blocks from `cargo fmt --check`: one warning per file.
    RustFmt,
    /// eslint `-f unix`: `path:line:col: message [Error/rule]`.
    EslintUnix,
    /// prettier `--check`: `[warn] path` lines.
    Prettier,
    /// rubocop `--format emacs`: `path:line:col: E|F|W|C|R: …` (E and F are errors).
    RubocopEmacs,
    /// One finding per `path:line:col:` line, all errors (go vet) or all warnings (ruff).
    Lines { errors: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tool {
    pub name: &'static str,
    pub program: &'static str,
    pub args: &'static [&'static str],
    pub parser: Parser,
}

/// The linters `project_root` is set up for.
pub fn detect(project_root: &Path) -> Vec<Tool> {
    let has = |file: &str| project_root.join(file).exists();
    let text = |file: &str| fs::read_to_string(project_root.join(file)).unwrap_or_default();
    let mut tools = Vec::new();
    if has("Cargo.toml") {
        tools.push(Tool {
            name: "clippy",
            program: "cargo",
            args: &["clippy", "--quiet", "--message-format=short"],
            parser: Parser::Rustc,
        });
        tools.push(Tool {
            name: "rustfmt",
            program: "cargo",
            args: &["fmt", "--check"],
            parser: Parser::RustFmt,
        });
    }
    let package = text("package.json");
    let eslint_config = [
        "eslint.config.js",
        "eslint.config.mjs",
        ".eslintrc",
        ".eslintrc.js",
        ".eslintrc.json",
    ];
    if package.contains("\"eslint\"") || eslint_config.iter().any(|f| has(f)) {
        tools.push(Tool {
            name: "eslint",
            program: "npx",
            args: &["--no-install", "eslint", ".", "-f", "unix"],
            parser: Parser::EslintUnix,
        });
    }
    if package.contains("\"prettier\"") || has(".prettierrc") {
        tools.push(Tool {
            name: "prettier",
            program: "npx",
            args: &["--no-install", "prettier", "--check", "."],
            parser: Parser::Prettier,
        });
    }
    if has(".rubocop.yml") || text("Gemfile").contains("rubocop") {
        tools.push(Tool {
            name: "rubocop",
            program: "bundle",
            args: &["exec", "rubocop", "--format", "emacs"],
            parser: Parser::RubocopEmacs,
        });
    }
    if text("pyproject.toml").contains("ruff") || has("ruff.toml") || has(".ruff.toml") {
        tools.push(Tool {
            name: "ruff",
            program: "ruff",
            args: &["check", "--output-format", "concise"],
            parser: Parser::Lines { errors: false },
        });
    }
    if has("go.mod") {
        tools.push(Tool {
            name: "go vet",
            program: "go",
            args: &["vet", "./..."],
            parser: Parser::Lines { errors: true },
        });
    }
    tools
}

/// (errors, warnings) in a tool's combined stdout + stderr.
pub fn count(parser: Parser, output: &str) -> (u32, u32) {
    let mut errors = 0;
    let mut warnings = 0;
    for line in output.lines() {
        let line = line.trim_end();
        match parser {
            Parser::Rustc => {
                if !has_location(line) {
                    continue;
                }
                if line.contains(": error") {
                    errors += 1;
                } else if line.contains(": warning") {
                    warnings += 1;
                }
            }
            Parser::RustFmt => warnings += u32::from(line.starts_with("Diff in ")),
            Parser::EslintUnix => {
                if line.ends_with(']') && line.contains("[Error/") {
                    errors += 1;
                } else if line.ends_with(']') && line.contains("[Warning/") {
                    warnings += 1;
                }
            }
            Parser::Prettier => {
                warnings += u32::from(line.starts_with("[warn] ") && !line.contains("Code style issues"));
            }
            Parser::RubocopEmacs => {
                if !has_location(line) {
                    continue;
                }
                if line.contains(": E: ") || line.contains(": F: ") {
                    errors += 1;
                } else if [": W: ", ": C: ", ": R: "].iter().any(|s| line.contains(s)) {
                    warnings += 1;
                }
            }
            Parser::Lines { errors: as_errors } => {
                if has_location(line) {
                    if as_errors {
                        errors += 1;
                    } else {
                        warnings += 1;
                    }
                }
            }
        }
    }
    (errors, warnings)
}

/// `path:line:col:` at the start of the line.
fn has_location(line: &str) -> bool {
    let mut parts = line.splitn(4, ':');
    let (Some(path), Some(row), Some(col), Some(_)) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    !path.is_empty() && row.trim().parse::<u32>().is_ok() && col.trim().parse::<u32>().is_ok()
}

/// 100 − error weight × errors − warning weight × warnings, clamped to 0…100.
pub fn score(errors: u32, warnings: u32, settings: &QualitySettings) -> f64 {
    let penalty = settings.error_weight * f64::from(errors) + settings.warning_weight * f64::from(warnings);
    (100.0 - penalty).clamp(0.0, 100.0)
}

pub fn sample(errors: u32, warnings: u32, settings: &QualitySettings) -> QualitySample {
    QualitySample {
        errors,
        warnings,
        score: score(errors, warnings, settings),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_project_tooling() {
        let dir = std::env::temp_dir().join(format!("pi-quality-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        assert!(detect(&dir).is_empty());
        fs::write(dir.join("Cargo.toml"), "[package]").unwrap();
        fs::write(
            dir.join("package.json"),
            r#"{"devDependencies":{"eslint":"9","prettier":"3"}}"#,
        )
        .unwrap();
        fs::write(dir.join("Gemfile"), "gem 'rubocop'").unwrap();
        fs::write(dir.join("go.mod"), "module x").unwrap();
        let names: Vec<&str> = detect(&dir).iter().map(|t| t.name).collect();
        assert_eq!(
            names,
            ["clippy", "rustfmt", "eslint", "prettier", "rubocop", "go vet"]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn counts_findings() {
        let clippy = "src/main.rs:3:9: warning: unused variable: `x`\n\
                      src/lib.rs:10:1: error[E0425]: cannot find value `y`\n\
                      warning: `app` (bin) generated 1 warning\nerror: could not compile `app`";
        assert_eq!(count(Parser::Rustc, clippy), (1, 1));
        assert_eq!(
            count(Parser::RustFmt, "Diff in /p/a.rs:1:\n-x\n+y\nDiff in /p/b.rs:2:"),
            (0, 2)
        );
        let eslint = "/p/a.js:1:7: 'x' is unused [Warning/no-unused-vars]\n/p/a.js:2:1: 'y' is undefined [Error/no-undef]\n\n2 problems";
        assert_eq!(count(Parser::EslintUnix, eslint), (1, 1));
        let prettier = "Checking formatting...\n[warn] src/a.ts\n[warn] Code style issues found in 1 file.";
        assert_eq!(count(Parser::Prettier, prettier), (0, 1));
        let rubocop = "app/a.rb:1:1: C: Style/Doc: Missing docs\napp/a.rb:3:5: E: Lint/Syntax: bad\napp/b.rb:2:2: W: Lint/Unused: x";
        assert_eq!(count(Parser::RubocopEmacs, rubocop), (1, 2));
        assert_eq!(
            count(Parser::Lines { errors: true }, "# pkg\na.go:3:2: printf bad\n"),
            (1, 0)
        );
        assert_eq!(
            count(
                Parser::Lines { errors: false },
                "a.py:1:1: F401 unused\nFound 1 error."
            ),
            (0, 1)
        );
    }

    #[test]
    fn scores_with_weights() {
        let settings = QualitySettings::default();
        assert!((score(0, 0, &settings) - 100.0).abs() < f64::EPSILON);
        assert!((score(2, 3, &settings) - 87.0).abs() < f64::EPSILON);
        assert!(score(50, 0, &settings).abs() < f64::EPSILON, "clamped at 0");
        assert_eq!(sample(1, 0, &settings).errors, 1);
    }
}
