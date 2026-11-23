//! Global rules: `rules/*.md` files with YAML front-matter (see `schemas/rule.schema.json`).
//! Matching a rule against a project and loading the folder with per-file errors.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use globset::{Glob, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::frontmatter;
use crate::project::ProjectFacts;

/// Where a rule applies: `all`, or any of the listed languages / path globs / projects
/// (a project matches when one criterion does).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum AppliesTo {
    #[default]
    All,
    Scoped(Scope),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    /// Language ids (`rust`, `typescript`, …), see `project::language_of`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<String>,
    /// Globs over project-relative file paths (`src/**/*.rs`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    /// Project roots (absolute, `~/` allowed) or globs over them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub projects: Vec<String>,
}

impl Serialize for AppliesTo {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::All => serializer.serialize_str("all"),
            Self::Scoped(scope) => scope.serialize(serializer),
        }
    }
}

// By hand, for errors that name the field (an untagged enum only says "no variant matched").
impl<'de> Deserialize<'de> for AppliesTo {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error as _;
        match Value::deserialize(deserializer)? {
            Value::String(s) if s == "all" => Ok(Self::All),
            Value::Object(map) => serde_json::from_value(Value::Object(map))
                .map(Self::Scoped)
                .map_err(|e| D::Error::custom(format!("`applies_to`: {e}"))),
            other => Err(D::Error::custom(format!(
                "`applies_to` must be \"all\" or a mapping with languages / paths / projects, got {other}"
            ))),
        }
    }
}

/// The front-matter of a rule file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleMeta {
    pub name: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub applies_to: AppliesTo,
    /// Higher comes first in the generated context.
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub tags: Vec<String>,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub meta: RuleMeta,
    /// Markdown instructions.
    pub body: String,
    /// The file it came from (`rules/<slug>.md`).
    pub path: PathBuf,
}

/// A rule file that could not be loaded, with the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadError {
    pub path: PathBuf,
    pub message: String,
}

impl Rule {
    /// Parses a rule file's text.
    pub fn parse(text: &str, path: impl Into<PathBuf>) -> Result<Self> {
        let doc = frontmatter::split(text);
        let meta_json = frontmatter::to_json(doc.front_matter)?;
        let meta: RuleMeta = serde_json::from_value(meta_json).context("front-matter")?;
        validate(&meta)?;
        Ok(Self {
            meta,
            body: doc.body.trim().to_string(),
            path: path.into(),
        })
    }

    /// The file text: front-matter in a fixed key order, then the body.
    pub fn render(&self) -> String {
        let meta = &self.meta;
        let mut fields = vec![
            ("name", json!(meta.name)),
            ("enabled", json!(meta.enabled)),
            ("description", json!(meta.description)),
            (
                "applies_to",
                serde_json::to_value(&meta.applies_to).unwrap_or(Value::Null),
            ),
            ("priority", json!(meta.priority)),
            ("tags", json!(meta.tags)),
        ];
        if meta.description.is_empty() {
            fields.retain(|(key, _)| *key != "description");
        }
        format!(
            "---\n{}---\n\n{}\n",
            frontmatter::render(&fields),
            self.body.trim()
        )
    }

    /// Whether the rule applies to `project` (enabled or not; see `matches_enabled`).
    pub fn matches(&self, project: &ProjectFacts) -> bool {
        let AppliesTo::Scoped(scope) = &self.meta.applies_to else {
            return true;
        };
        let language = scope
            .languages
            .iter()
            .any(|l| project.languages.contains(&l.to_ascii_lowercase()));
        language || matches_paths(&scope.paths, project) || matches_project(&scope.projects, project)
    }

    pub fn matches_enabled(&self, project: &ProjectFacts) -> bool {
        self.meta.enabled && self.matches(project)
    }

    /// File name stem used as the rule's id.
    pub fn id(&self) -> String {
        self.path
            .file_stem()
            .map_or_else(String::new, |s| s.to_string_lossy().into_owned())
    }
}

fn validate(meta: &RuleMeta) -> Result<()> {
    if meta.name.trim().is_empty() {
        bail!("`name` must not be empty");
    }
    if let AppliesTo::Scoped(scope) = &meta.applies_to {
        if scope.languages.is_empty() && scope.paths.is_empty() && scope.projects.is_empty() {
            bail!("`applies_to` needs `languages`, `paths` or `projects` (or use `applies_to: all`)");
        }
        for glob in scope.paths.iter().chain(&scope.projects) {
            Glob::new(glob).with_context(|| format!("bad glob {glob:?} in `applies_to`"))?;
        }
    }
    Ok(())
}

fn matches_paths(globs: &[String], project: &ProjectFacts) -> bool {
    if globs.is_empty() {
        return false;
    }
    let mut builder = GlobSetBuilder::new();
    for glob in globs {
        if let Ok(glob) = Glob::new(glob) {
            builder.add(glob);
        }
    }
    let Ok(set) = builder.build() else { return false };
    project.files.iter().any(|file| set.is_match(file))
}

fn matches_project(patterns: &[String], project: &ProjectFacts) -> bool {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    patterns.iter().any(|pattern| {
        let expanded = match (pattern.strip_prefix("~/"), &home) {
            (Some(rest), Some(home)) => home.join(rest).to_string_lossy().into_owned(),
            _ => pattern.clone(),
        };
        let expanded = expanded.trim_end_matches('/');
        Path::new(expanded) == project.root
            || Glob::new(expanded).is_ok_and(|glob| glob.compile_matcher().is_match(&project.root))
    })
}

/// Loads every `*.md` in `dir`, sorted by priority (highest first), then name.
/// Files that fail are reported, not fatal.
pub fn load_dir(dir: &Path) -> (Vec<Rule>, Vec<LoadError>) {
    let mut rules = Vec::new();
    let mut errors = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return (rules, errors);
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        .collect();
    paths.sort();
    for path in paths {
        let parsed = fs::read_to_string(&path)
            .context("reading the file")
            .and_then(|text| Rule::parse(&text, &path));
        match parsed {
            Ok(rule) => rules.push(rule),
            Err(err) => errors.push(LoadError {
                path,
                message: format!("{err:#}"),
            }),
        }
    }
    sort(&mut rules);
    (rules, errors)
}

/// Highest priority first; ties by name, so the order is stable.
pub fn sort(rules: &mut [Rule]) {
    rules.sort_by(|a, b| {
        b.meta
            .priority
            .cmp(&a.meta.priority)
            .then_with(|| a.meta.name.cmp(&b.meta.name))
    });
}

/// A file-system-safe id from a rule name (`Rust style` → `rust-style`).
pub fn slug(name: &str) -> String {
    let mut slug = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    let slug = slug.trim_end_matches('-').to_string();
    if slug.is_empty() { "rule".into() } else { slug }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUST_RULE: &str = "---\nname: Rust style\ndescription: How we write Rust\n\
        applies_to:\n  languages: [rust]\npriority: 10\ntags: [style]\n---\n\nUse `let … else`.\n";

    fn project() -> ProjectFacts {
        ProjectFacts::new("/work/app", vec!["src/main.rs".into(), "web/index.ts".into()])
    }

    #[test]
    fn parses_a_rule() {
        let rule = Rule::parse(RUST_RULE, "rules/rust-style.md").unwrap();
        assert_eq!(rule.meta.name, "Rust style");
        assert!(rule.meta.enabled, "enabled defaults to true");
        assert_eq!(rule.meta.priority, 10);
        assert_eq!(rule.body, "Use `let … else`.");
        assert_eq!(rule.id(), "rust-style");
        assert_eq!(
            rule.meta.applies_to,
            AppliesTo::Scoped(Scope {
                languages: vec!["rust".into()],
                ..Scope::default()
            })
        );
    }

    #[test]
    fn reports_bad_front_matter() {
        let err = |text: &str| format!("{:#}", Rule::parse(text, "r.md").unwrap_err());
        assert!(err("---\nname: A\nprio: 1\n---\nx").contains("unknown field `prio`"));
        assert!(err("---\ndescription: no name\n---\nx").contains("missing field `name`"));
        assert!(err("---\nname: A\napplies_to: some\n---\nx").contains("applies_to"));
        assert!(err("---\nname: A\napplies_to: {}\n---\nx").contains("needs `languages`"));
        assert!(err("---\nname: A\napplies_to:\n  paths: [\"a[\"]\n---\nx").contains("bad glob"));
        assert!(err("---\nname: \" \"\n---\nx").contains("must not be empty"));
    }

    #[test]
    fn matches_projects() {
        let rule = |applies_to: &str| {
            Rule::parse(
                &format!("---\nname: R\napplies_to: {applies_to}\n---\nbody"),
                "r.md",
            )
            .unwrap()
        };
        let p = project();
        assert!(rule("all").matches(&p));
        assert!(
            rule("{ languages: [Rust] }").matches(&p),
            "language ids ignore case"
        );
        assert!(!rule("{ languages: [python] }").matches(&p));
        assert!(rule("{ paths: [\"web/**/*.ts\"] }").matches(&p));
        assert!(!rule("{ paths: [\"docs/**\"] }").matches(&p));
        assert!(rule("{ projects: [\"/work/app/\"] }").matches(&p));
        assert!(rule("{ projects: [\"/work/*\"] }").matches(&p));
        assert!(!rule("{ projects: [\"/other\"] }").matches(&p));
        assert!(
            rule("{ languages: [python], paths: [\"src/*.rs\"] }").matches(&p),
            "any criterion"
        );

        let mut disabled = rule("all");
        disabled.meta.enabled = false;
        assert!(disabled.matches(&p) && !disabled.matches_enabled(&p));
    }

    #[test]
    fn renders_and_round_trips() {
        let rule = Rule::parse(RUST_RULE, "rules/rust-style.md").unwrap();
        let text = rule.render();
        assert!(
            text.starts_with("---\nname: \"Rust style\"\nenabled: true\n"),
            "{text}"
        );
        assert_eq!(Rule::parse(&text, "rules/rust-style.md").unwrap(), rule);

        let all = Rule::parse("---\nname: Everywhere\n---\nx", "e.md").unwrap();
        assert!(all.render().contains("applies_to: \"all\"\n"));
        assert_eq!(Rule::parse(&all.render(), "e.md").unwrap(), all);
    }

    #[test]
    fn loads_a_folder_sorted_with_errors() {
        let dir = std::env::temp_dir().join(format!("pi-rules-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.md"), "---\nname: Low\npriority: 1\n---\nx").unwrap();
        fs::write(dir.join("b.md"), "---\nname: High\npriority: 9\n---\ny").unwrap();
        fs::write(dir.join("c.md"), "---\nname: [broken\n---\n").unwrap();
        fs::write(dir.join("notes.txt"), "ignored").unwrap();
        let (rules, errors) = load_dir(&dir);
        let names: Vec<&str> = rules.iter().map(|r| r.meta.name.as_str()).collect();
        assert_eq!(names, ["High", "Low"]);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].path.ends_with("c.md"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn makes_slugs() {
        assert_eq!(slug("Rust style!"), "rust-style");
        assert_eq!(slug("  Тест  "), "rule");
        assert_eq!(slug("A/B c"), "a-b-c");
    }
}
