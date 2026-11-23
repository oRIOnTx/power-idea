//! Starting `claude` for a project: the command line, its generated files (rules, session
//! settings with hooks and status line, MCP config, skills plugin) and a readable preview.
//! `plan` is pure; `write` puts the files in the cache folder.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde_json::{Value, json};

use crate::resolver::{self, Effective};
use crate::settings::ClaudeSettings;
use crate::skills;

/// Environment variables the hook / status-line helpers read to reach the IDE.
pub const SOCKET_ENV: &str = "POWER_IDEA_CLAUDE_SOCKET";
pub const TAG_ENV: &str = "POWER_IDEA_CLAUDE_TAG";

/// Hook events forwarded to the IDE (all documented, see docs/claude-code-integration.md).
pub const HOOK_EVENTS: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "Notification",
    "Stop",
    "SessionEnd",
];
const TOOL_EVENTS: &[&str] = &["PreToolUse", "PostToolUse"];

/// The IDE's MCP server, for `--mcp-config`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpEndpoint {
    pub port: u16,
    pub token: String,
}

/// What the caller knows beyond the settings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchOptions {
    /// The IDE executable, which also serves `claude-hook` / `claude-statusline`.
    pub ide_exe: Option<PathBuf>,
    /// The bridge socket the helpers write to.
    pub socket: Option<PathBuf>,
    /// Routes bridge messages to the window that started the session.
    pub tag: String,
    pub mcp: Option<McpEndpoint>,
    /// The status line command the user configured, run by ours so theirs keeps showing.
    pub user_statusline: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedFile {
    pub path: PathBuf,
    pub contents: String,
    /// Holds a secret: written with mode 0600 and redacted in previews.
    pub private: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: PathBuf,
    pub files: Vec<GeneratedFile>,
    /// The session plugin folder and the skill folders to copy into it.
    pub plugin: Option<(PathBuf, Vec<PathBuf>)>,
    /// Skill names in the plugin (for the preview and the UI).
    pub skill_names: Vec<String>,
}

/// Builds the launch for `project_root`, with generated files under `cache_dir`.
pub fn plan(
    settings: &ClaudeSettings,
    effective: &Effective<'_>,
    program: &Path,
    project_root: &Path,
    cache_dir: &Path,
    continue_last: bool,
    options: &LaunchOptions,
) -> LaunchPlan {
    let mut plan = LaunchPlan {
        program: program.to_path_buf(),
        args: Vec::new(),
        env: Vec::new(),
        cwd: project_root.to_path_buf(),
        files: Vec::new(),
        plugin: None,
        skill_names: Vec::new(),
    };
    if !settings.enabled {
        if continue_last {
            plan.args.push("--continue".into());
        }
        return plan;
    }

    if resolver::injects_rules(settings) && !effective.rules_text.is_empty() {
        let path = cache_dir.join("rules.md");
        plan.args
            .extend(["--append-system-prompt-file".into(), path_arg(&path)]);
        plan.files.push(GeneratedFile {
            path,
            contents: effective.rules_text.clone(),
            private: false,
        });
    }

    if resolver::injects_skills(settings) && !effective.skills.is_empty() {
        let dir = cache_dir.join("plugin").join(skills::PLUGIN_NAME);
        plan.args.extend(["--plugin-dir".into(), path_arg(&dir)]);
        plan.skill_names = effective.skills.iter().map(|s| s.name.clone()).collect();
        plan.plugin = Some((dir, effective.skills.iter().map(|s| s.dir.clone()).collect()));
    }

    if let Some(session) = session_settings(settings, cache_dir, options) {
        let path = cache_dir.join("settings.json");
        plan.args.extend(["--settings".into(), path_arg(&path)]);
        plan.files.push(GeneratedFile {
            path,
            contents: pretty(&session),
            private: false,
        });
        if let Some(user) = &options.user_statusline {
            plan.files.push(GeneratedFile {
                path: cache_dir.join("user-statusline"),
                contents: user.clone(),
                private: false,
            });
        }
        if let Some(socket) = &options.socket {
            plan.env
                .push((SOCKET_ENV.into(), socket.to_string_lossy().into_owned()));
        }
        plan.env.push((TAG_ENV.into(), options.tag.clone()));
    }

    if settings.general.ide_mcp
        && let Some(mcp) = &options.mcp
    {
        let path = cache_dir.join("mcp.json");
        let config = json!({ "mcpServers": { "power-idea": {
            "type": "http",
            "url": format!("http://127.0.0.1:{}/mcp", mcp.port),
            "headers": { "Authorization": format!("Bearer {}", mcp.token) },
        }}});
        plan.args.extend(["--mcp-config".into(), path_arg(&path)]);
        plan.files.push(GeneratedFile {
            path,
            contents: pretty(&config),
            private: true,
        });
    }

    let general = &settings.general;
    for (flag, value) in [
        ("--model", &general.model),
        ("--permission-mode", &general.permission_mode),
        ("--effort", &general.effort),
    ] {
        if !value.trim().is_empty() {
            plan.args.extend([flag.to_string(), value.trim().to_string()]);
        }
    }
    if continue_last {
        plan.args.push("--continue".into());
    }
    plan
}

/// Hooks and status line for the session, when gauges or quality need live events.
fn session_settings(settings: &ClaudeSettings, cache_dir: &Path, options: &LaunchOptions) -> Option<Value> {
    let live = settings.gauges.enabled || settings.quality.enabled;
    let exe = options.ide_exe.as_ref().filter(|_| live)?;
    let hook = format!("{} claude-hook", shell_quote(&exe.to_string_lossy()));
    let mut hooks = serde_json::Map::new();
    for event in HOOK_EVENTS {
        // The bridge exits within a second; the timeout only guards against the unexpected.
        let handler = json!({ "type": "command", "command": hook, "timeout": 5 });
        let group = if TOOL_EVENTS.contains(event) {
            json!({ "matcher": "*", "hooks": [handler] })
        } else {
            json!({ "hooks": [handler] })
        };
        hooks.insert((*event).to_string(), json!([group]));
    }
    let mut statusline = format!("{} claude-statusline", shell_quote(&exe.to_string_lossy()));
    if options.user_statusline.is_some() {
        let file = cache_dir.join("user-statusline");
        let _ = write!(
            statusline,
            " --user-command-file {}",
            shell_quote(&file.to_string_lossy())
        );
    }
    Some(json!({
        "hooks": hooks,
        "statusLine": { "type": "command", "command": statusline },
    }))
}

/// The status line command the user set, by Claude Code's precedence (local > project > user).
pub fn user_statusline(project_root: &Path, home: &Path) -> Option<String> {
    let files = [
        project_root.join(".claude/settings.local.json"),
        project_root.join(".claude/settings.json"),
        home.join(".claude/settings.json"),
    ];
    files.iter().find_map(|path| {
        let value: Value = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
        let status = value.get("statusLine")?;
        (status.get("type").and_then(Value::as_str) == Some("command"))
            .then(|| status.get("command").and_then(Value::as_str).map(str::to_string))
            .flatten()
            .filter(|c| !c.contains("claude-statusline"))
    })
}

/// Writes the generated files and the skills plugin.
pub fn write(plan: &LaunchPlan) -> Result<()> {
    for file in &plan.files {
        if let Some(dir) = file.path.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(&file.path, &file.contents).with_context(|| format!("writing {}", file.path.display()))?;
        if file.private {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&file.path, fs::Permissions::from_mode(0o600))?;
        }
    }
    if let Some((dir, skill_dirs)) = &plan.plugin {
        skills::write_plugin(dir, skill_dirs)?;
    }
    Ok(())
}

/// What "Preview effective context" shows: the command line and every generated file,
/// secrets redacted.
pub fn preview(plan: &LaunchPlan) -> String {
    let mut out = String::new();
    let command: Vec<String> = std::iter::once(plan.program.to_string_lossy().into_owned())
        .chain(plan.args.iter().cloned())
        .map(|a| shell_quote(&a))
        .collect();
    let _ = writeln!(out, "Working directory: {}\n", plan.cwd.display());
    let _ = writeln!(out, "Command:\n  {}\n", command.join(" "));
    if !plan.env.is_empty() {
        out.push_str("Environment:\n");
        for (key, value) in &plan.env {
            let _ = writeln!(out, "  {key}={value}");
        }
        out.push('\n');
    }
    if !plan.skill_names.is_empty() {
        let _ = writeln!(
            out,
            "Skills (plugin `{}`, invoked as /{}:<name>): {}\n",
            skills::PLUGIN_NAME,
            skills::PLUGIN_NAME,
            plan.skill_names.join(", ")
        );
    }
    for file in &plan.files {
        let contents = if file.private {
            redact(&file.contents)
        } else {
            file.contents.clone()
        };
        let _ = writeln!(out, "── {} ──\n{}\n", file.path.display(), contents.trim_end());
    }
    if plan.args.is_empty() {
        out.push_str("Nothing is injected: claude starts exactly as from a terminal.\n");
    }
    out
}

fn redact(text: &str) -> String {
    text.lines()
        .map(|line| match line.split_once("Bearer ") {
            Some((before, _)) => format!("{before}Bearer ••••••\""),
            None => line.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

fn path_arg(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// POSIX shell quoting for display and hook commands.
pub fn shell_quote(text: &str) -> String {
    let safe = !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:=@%+,".contains(c));
    if safe {
        text.to_string()
    } else {
        format!("'{}'", text.replace('\'', r"'\''"))
    }
}

/// Finds the `claude` binary: the configured path, else PATH, else the usual install places.
pub fn locate_claude(configured: &str, path_env: Option<&str>, home: &Path) -> Option<PathBuf> {
    let configured = configured.trim();
    if !configured.is_empty() {
        let path = configured
            .strip_prefix("~/")
            .map_or_else(|| PathBuf::from(configured), |rest| home.join(rest));
        return path.is_file().then_some(path);
    }
    let from_path = path_env
        .into_iter()
        .flat_map(|p| p.split(':'))
        .filter(|d| !d.is_empty())
        .map(|d| Path::new(d).join("claude"));
    let usual = [
        home.join(".claude/local/claude"),
        home.join(".local/bin/claude"),
        PathBuf::from("/opt/homebrew/bin/claude"),
        PathBuf::from("/usr/local/bin/claude"),
    ];
    from_path.chain(usual).find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::ProjectFacts;
    use crate::rules::Rule;
    use crate::skills::Skill;

    fn on() -> ClaudeSettings {
        let mut settings = ClaudeSettings {
            enabled: true,
            ..ClaudeSettings::default()
        };
        settings.gauges.enabled = false;
        settings
    }

    fn rules() -> Vec<Rule> {
        vec![Rule::parse("---\nname: Style\n---\nBe nice.", "rules/style.md").unwrap()]
    }

    fn plan_for(settings: &ClaudeSettings, options: &LaunchOptions, continue_last: bool) -> LaunchPlan {
        let rules = rules();
        let skills = [Skill {
            name: "review".into(),
            description: String::new(),
            dir: PathBuf::from("/lib/skills/review"),
            builtin: true,
        }];
        let effective = resolver::resolve(settings, &rules, &skills, &ProjectFacts::new("/p", vec![]));
        plan(
            settings,
            &effective,
            Path::new("/bin/claude"),
            Path::new("/p"),
            Path::new("/cache/p1"),
            continue_last,
            options,
        )
    }

    #[test]
    fn disabled_section_launches_plain_claude() {
        let plan = plan_for(&ClaudeSettings::default(), &LaunchOptions::default(), true);
        assert_eq!(plan.args, ["--continue"]);
        assert!(plan.files.is_empty() && plan.env.is_empty() && plan.plugin.is_none());
        assert!(preview(&plan).contains("--continue"));
    }

    #[test]
    fn injects_rules_skills_and_flags() {
        let mut settings = on();
        settings.general.model = "opus".into();
        settings.general.permission_mode = "plan".into();
        let plan = plan_for(&settings, &LaunchOptions::default(), false);
        assert_eq!(
            plan.args,
            [
                "--append-system-prompt-file",
                "/cache/p1/rules.md",
                "--plugin-dir",
                "/cache/p1/plugin/power-idea",
                "--model",
                "opus",
                "--permission-mode",
                "plan",
            ]
        );
        assert!(plan.files[0].contents.contains("## Style\n\nBe nice."));
        assert_eq!(plan.skill_names, ["review"]);
        assert!(plan.env.is_empty(), "no hooks without gauges or quality");
    }

    #[test]
    fn adds_hooks_and_status_line_for_live_gauges() {
        let mut settings = on();
        settings.gauges.enabled = true;
        settings.skills.enabled = false;
        let options = LaunchOptions {
            ide_exe: Some(PathBuf::from(
                "/Applications/Power IDEA.app/Contents/MacOS/power-idea",
            )),
            socket: Some(PathBuf::from("/tmp/ide.sock")),
            tag: "w1".into(),
            user_statusline: Some("~/bin/status.sh".into()),
            ..LaunchOptions::default()
        };
        let plan = plan_for(&settings, &options, false);
        let settings_file = plan
            .files
            .iter()
            .find(|f| f.path.ends_with("settings.json"))
            .unwrap();
        let session: Value = serde_json::from_str(&settings_file.contents).unwrap();
        let hook = &session["hooks"]["PostToolUse"][0];
        assert_eq!(hook["matcher"], "*");
        assert_eq!(
            hook["hooks"][0]["command"],
            "'/Applications/Power IDEA.app/Contents/MacOS/power-idea' claude-hook"
        );
        assert!(session["hooks"]["Stop"][0].get("matcher").is_none());
        let status = session["statusLine"]["command"].as_str().unwrap();
        assert!(
            status.ends_with("claude-statusline --user-command-file /cache/p1/user-statusline"),
            "{status}"
        );
        assert!(
            plan.files
                .iter()
                .any(|f| f.path.ends_with("user-statusline") && f.contents == "~/bin/status.sh")
        );
        assert_eq!(
            plan.env,
            [
                (SOCKET_ENV.to_string(), "/tmp/ide.sock".to_string()),
                (TAG_ENV.to_string(), "w1".to_string())
            ]
        );
    }

    #[test]
    fn mcp_config_is_private_and_redacted() {
        let mut settings = on();
        settings.general.ide_mcp = true;
        let options = LaunchOptions {
            mcp: Some(McpEndpoint {
                port: 7777,
                token: "s3cret".into(),
            }),
            ..LaunchOptions::default()
        };
        let plan = plan_for(&settings, &options, false);
        let mcp = plan.files.iter().find(|f| f.path.ends_with("mcp.json")).unwrap();
        assert!(mcp.private && mcp.contents.contains("Bearer s3cret"));
        let shown = preview(&plan);
        assert!(
            !shown.contains("s3cret") && shown.contains("Bearer ••••••"),
            "{shown}"
        );
    }

    #[test]
    fn writes_files_and_plugin() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = std::env::temp_dir().join(format!("pi-launch-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let skill = dir.join("lib/review");
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), "---\ndescription: R\n---\n").unwrap();
        let plan = LaunchPlan {
            program: PathBuf::from("claude"),
            args: vec![],
            env: vec![],
            cwd: dir.clone(),
            files: vec![GeneratedFile {
                path: dir.join("cache/mcp.json"),
                contents: "{}".into(),
                private: true,
            }],
            plugin: Some((dir.join("cache/plugin/power-idea"), vec![skill])),
            skill_names: vec!["review".into()],
        };
        write(&plan).unwrap();
        let mode = fs::metadata(dir.join("cache/mcp.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        assert!(
            dir.join("cache/plugin/power-idea/skills/review/SKILL.md")
                .is_file()
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_the_users_status_line() {
        let dir = std::env::temp_dir().join(format!("pi-status-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let (project, home) = (dir.join("p"), dir.join("home"));
        fs::create_dir_all(project.join(".claude")).unwrap();
        fs::create_dir_all(home.join(".claude")).unwrap();
        fs::write(
            home.join(".claude/settings.json"),
            r#"{"statusLine":{"type":"command","command":"user.sh"}}"#,
        )
        .unwrap();
        assert_eq!(user_statusline(&project, &home).as_deref(), Some("user.sh"));
        fs::write(
            project.join(".claude/settings.json"),
            r#"{"statusLine":{"type":"command","command":"project.sh"}}"#,
        )
        .unwrap();
        assert_eq!(user_statusline(&project, &home).as_deref(), Some("project.sh"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn quotes_and_locates() {
        assert_eq!(shell_quote("plain/path-1"), "plain/path-1");
        assert_eq!(shell_quote("it's here"), r"'it'\''s here'");
        assert_eq!(shell_quote(""), "''");
        let home = std::env::temp_dir();
        assert_eq!(locate_claude("/no/such/claude", None, &home), None);
        assert_eq!(
            locate_claude("/bin/sh", None, &home),
            Some(PathBuf::from("/bin/sh"))
        );
    }
}
