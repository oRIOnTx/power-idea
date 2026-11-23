//! The effective Claude Code context for a project: which rules and skills apply, and the
//! rules text that is injected (or installed). Pure: no I/O.

use std::fmt::Write as _;

use crate::project::ProjectFacts;
use crate::rules::{AppliesTo, Rule};
use crate::settings::{ClaudeSettings, Mode};
use crate::skills::{self, Skill};

/// Stated in the injected text, so the documented precedence holds: project instructions win.
pub const PRECEDENCE_NOTE: &str = "These rules come from the Power IDEA IDE settings. The project's own \
    CLAUDE.md files and .claude settings take precedence: where they conflict with a rule below, follow the \
    project.";

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Effective<'a> {
    /// Enabled rules that match the project, highest priority first.
    pub rules: Vec<&'a Rule>,
    /// Enabled skills.
    pub skills: Vec<&'a Skill>,
    /// What `--append-system-prompt-file` carries; empty when no rule applies.
    pub rules_text: String,
}

/// Resolves `settings` for `project`. With the section (or a feature) off, that part is empty.
pub fn resolve<'a>(
    settings: &ClaudeSettings,
    rules: &'a [Rule],
    skills: &'a [Skill],
    project: &ProjectFacts,
) -> Effective<'a> {
    if !settings.enabled {
        return Effective::default();
    }
    let rules: Vec<&Rule> = if settings.rules.enabled {
        let mut matching: Vec<&Rule> = rules.iter().filter(|r| r.matches_enabled(project)).collect();
        matching.sort_by(|a, b| {
            b.meta
                .priority
                .cmp(&a.meta.priority)
                .then_with(|| a.meta.name.cmp(&b.meta.name))
        });
        matching
    } else {
        Vec::new()
    };
    let skills = if settings.skills.enabled {
        skills::enabled(skills, &settings.skills.disabled)
    } else {
        Vec::new()
    };
    Effective {
        rules_text: rules_text(&rules, false),
        rules,
        skills,
    }
}

/// Whether rules / skills travel as CLI flags (inject) for this launch.
pub fn injects_rules(settings: &ClaudeSettings) -> bool {
    settings.enabled && settings.rules.enabled && settings.rules.mode == Mode::Inject
}

pub fn injects_skills(settings: &ClaudeSettings) -> bool {
    settings.enabled && settings.skills.enabled && settings.skills.mode == Mode::Inject
}

/// The Markdown sent to Claude Code. `with_scopes` (install mode, where the IDE cannot match per
/// project) states each scoped rule's scope so Claude can tell when it applies.
pub fn rules_text(rules: &[&Rule], with_scopes: bool) -> String {
    if rules.is_empty() {
        return String::new();
    }
    let mut text = format!("# Power IDEA rules\n\n{PRECEDENCE_NOTE}\n");
    for rule in rules {
        text.push_str("\n## ");
        text.push_str(rule.meta.name.trim());
        text.push('\n');
        if with_scopes && let AppliesTo::Scoped(scope) = &rule.meta.applies_to {
            let mut parts = Vec::new();
            if !scope.languages.is_empty() {
                parts.push(format!("{} code", scope.languages.join(", ")));
            }
            if !scope.paths.is_empty() {
                parts.push(format!("files matching {}", scope.paths.join(", ")));
            }
            if !scope.projects.is_empty() {
                parts.push(format!("the projects {}", scope.projects.join(", ")));
            }
            let _ = write!(
                text,
                "
_Applies only to {}._
",
                parts.join("; or ")
            );
        }
        text.push('\n');
        text.push_str(rule.body.trim());
        text.push('\n');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn rule(name: &str, priority: i32, applies_to: &str, enabled: bool) -> Rule {
        let text = format!(
            "---\nname: {name}\nenabled: {enabled}\npriority: {priority}\napplies_to: {applies_to}\n---\nBody of {name}."
        );
        Rule::parse(&text, format!("rules/{name}.md")).unwrap()
    }

    fn skill(name: &str) -> Skill {
        Skill {
            name: name.into(),
            description: String::new(),
            dir: PathBuf::from(name),
            builtin: false,
        }
    }

    fn on() -> ClaudeSettings {
        ClaudeSettings {
            enabled: true,
            ..ClaudeSettings::default()
        }
    }

    #[test]
    fn picks_matching_rules_by_priority() {
        let rules = [
            rule("Low", 1, "all", true),
            rule("Rust", 5, "{ languages: [rust] }", true),
            rule("Python", 9, "{ languages: [python] }", true),
            rule("Off", 99, "all", false),
        ];
        let project = ProjectFacts::new("/p", vec!["main.rs".into()]);
        let effective = resolve(&on(), &rules, &[], &project);
        let names: Vec<&str> = effective.rules.iter().map(|r| r.meta.name.as_str()).collect();
        assert_eq!(names, ["Rust", "Low"]);
        assert!(
            effective
                .rules_text
                .starts_with("# Power IDEA rules\n\nThese rules come from")
        );
        assert!(effective.rules_text.find("## Rust") < effective.rules_text.find("## Low"));
        assert!(!effective.rules_text.contains("Python") && !effective.rules_text.contains("Off"));
    }

    #[test]
    fn everything_off_means_nothing() {
        let rules = [rule("A", 0, "all", true)];
        let skills = [skill("s")];
        let project = ProjectFacts::default();
        assert_eq!(
            resolve(&ClaudeSettings::default(), &rules, &skills, &project),
            Effective::default()
        );

        let mut settings = on();
        settings.rules.enabled = false;
        settings.skills.disabled = vec!["s".into()];
        let effective = resolve(&settings, &rules, &skills, &project);
        assert!(effective.rules.is_empty() && effective.rules_text.is_empty() && effective.skills.is_empty());
    }

    #[test]
    fn modes_decide_injection() {
        let mut settings = on();
        assert!(injects_rules(&settings) && injects_skills(&settings));
        settings.rules.mode = Mode::Install;
        assert!(!injects_rules(&settings));
        settings.enabled = false;
        assert!(!injects_skills(&settings));
    }

    #[test]
    fn states_scopes_for_install_mode() {
        let rules = [rule(
            "Rust",
            0,
            "{ languages: [rust], paths: [\"src/**\"] }",
            true,
        )];
        let refs: Vec<&Rule> = rules.iter().collect();
        let text = rules_text(&refs, true);
        assert!(
            text.contains("_Applies only to rust code; or files matching src/**._"),
            "{text}"
        );
        assert!(!rules_text(&refs, false).contains("Applies only"));
    }
}
