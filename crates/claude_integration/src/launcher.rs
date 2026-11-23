//! Starting `claude` for a project: the command line and its generated files (session settings
//! with hooks and status line, MCP config). `plan` is pure; `write` puts the files in the cache
//! folder.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde_json::{Value, json};

use crate::settings::ClaudeSettings;

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
    /// Holds a secret: written with mode 0600.
    pub private: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: PathBuf,
    pub files: Vec<GeneratedFile>,
}

/// How a Claude tab starts its session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Start {
    #[default]
    New,
    /// `--continue`: the most recent session in the project.
    Continue,
    /// `--resume`: Claude Code's own picker of earlier sessions.
    Resume,
}

impl Start {
    fn flag(self) -> Option<&'static str> {
        match self {
            Self::New => None,
            Self::Continue => Some("--continue"),
            Self::Resume => Some("--resume"),
        }
    }
}

/// Builds the launch for `project_root`, with generated files under `cache_dir`.
pub fn plan(
    settings: &ClaudeSettings,
    program: &Path,
    project_root: &Path,
    cache_dir: &Path,
    start: Start,
    options: &LaunchOptions,
) -> LaunchPlan {
    let mut plan = LaunchPlan {
        program: program.to_path_buf(),
        args: Vec::new(),
        env: Vec::new(),
        cwd: project_root.to_path_buf(),
        files: Vec::new(),
    };
    if settings.enabled {
        add_integration(&mut plan, settings, cache_dir, options);
    }
    plan.args.extend(start.flag().map(String::from));
    plan
}

/// The IDE's additions: hooks and status line for the live gauges, and its MCP server.
fn add_integration(
    plan: &mut LaunchPlan,
    settings: &ClaudeSettings,
    cache_dir: &Path,
    options: &LaunchOptions,
) {
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
}

/// Hooks and status line for the session, when the live gauges need its events.
fn session_settings(settings: &ClaudeSettings, cache_dir: &Path, options: &LaunchOptions) -> Option<Value> {
    let exe = options.ide_exe.as_ref().filter(|_| settings.gauges.enabled)?;
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

/// Writes the generated files.
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
    Ok(())
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

fn path_arg(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// A stable folder name for a project's generated files (FNV-1a of the root path).
pub fn cache_key(project_root: &Path) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in project_root.to_string_lossy().bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    let name = project_root
        .file_name()
        .map_or_else(String::new, |n| slug(&n.to_string_lossy()));
    format!("{name}-{hash:016x}")
}

/// Lowercase ASCII letters and digits, other runs collapsed to one `-`.
fn slug(name: &str) -> String {
    let mut slug = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() {
        "project".into()
    } else {
        slug.into()
    }
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

    fn on() -> ClaudeSettings {
        let mut settings = ClaudeSettings {
            enabled: true,
            ..ClaudeSettings::default()
        };
        settings.gauges.enabled = false;
        settings
    }

    fn plan_for(settings: &ClaudeSettings, options: &LaunchOptions, start: Start) -> LaunchPlan {
        plan(
            settings,
            Path::new("/bin/claude"),
            Path::new("/p"),
            Path::new("/cache/p1"),
            start,
            options,
        )
    }

    #[test]
    fn disabled_section_launches_plain_claude() {
        let options = LaunchOptions {
            ide_exe: Some(PathBuf::from("/ide")),
            ..LaunchOptions::default()
        };
        let plan = plan_for(&ClaudeSettings::default(), &options, Start::Continue);
        assert_eq!(plan.args, ["--continue"]);
        assert!(plan.files.is_empty() && plan.env.is_empty());
    }

    #[test]
    fn start_modes_add_their_flag_last() {
        let plain = plan_for(&on(), &LaunchOptions::default(), Start::New);
        assert!(plain.args.is_empty(), "nothing injected without gauges or MCP");
        assert!(plain.env.is_empty());
        let resume = plan_for(&on(), &LaunchOptions::default(), Start::Resume);
        assert_eq!(resume.args, ["--resume"]);
    }

    #[test]
    fn adds_hooks_and_status_line_for_live_gauges() {
        let mut settings = on();
        settings.gauges.enabled = true;
        let options = LaunchOptions {
            ide_exe: Some(PathBuf::from(
                "/Applications/Power IDEA.app/Contents/MacOS/power-idea",
            )),
            socket: Some(PathBuf::from("/tmp/ide.sock")),
            tag: "w1".into(),
            user_statusline: Some("~/bin/status.sh".into()),
            ..LaunchOptions::default()
        };
        let plan = plan_for(&settings, &options, Start::New);
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
        let plan = plan_for(&settings, &options, Start::New);
        let mcp = plan.files.iter().find(|f| f.path.ends_with("mcp.json")).unwrap();
        assert!(mcp.private && mcp.contents.contains("Bearer s3cret"));
        assert_eq!(plan.args, ["--mcp-config", "/cache/p1/mcp.json"]);
    }

    #[test]
    fn writes_private_files_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = std::env::temp_dir().join(format!("pi-launch-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
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
        };
        write(&plan).unwrap();
        let mode = fs::metadata(dir.join("cache/mcp.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
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
    fn makes_stable_cache_keys() {
        let a = cache_key(Path::new("/work/My App"));
        assert_eq!(a, cache_key(Path::new("/work/My App")));
        assert!(a.starts_with("my-app-"));
        assert_ne!(a, cache_key(Path::new("/other/My App")));
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
