//! Starting `claude` for a project: the command line and its generated files (MCP config).
//! `plan` is pure; `write` puts the files in the cache folder.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde_json::{Value, json};

use crate::settings::ClaudeSettings;

/// The IDE's MCP server, for `--mcp-config`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpEndpoint {
    pub port: u16,
    pub token: String,
}

/// What the caller knows beyond the settings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchOptions {
    pub mcp: Option<McpEndpoint>,
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
        add_mcp(&mut plan, settings, cache_dir, options);
    }
    plan.args.extend(start.flag().map(String::from));
    plan
}

/// The IDE's MCP server, when enabled and running.
fn add_mcp(plan: &mut LaunchPlan, settings: &ClaudeSettings, cache_dir: &Path, options: &LaunchOptions) {
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

/// POSIX shell quoting for displayed command lines.
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
        ClaudeSettings {
            enabled: true,
            ..ClaudeSettings::default()
        }
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
            mcp: Some(McpEndpoint {
                port: 7777,
                token: "t".into(),
            }),
        };
        let plan = plan_for(&ClaudeSettings::default(), &options, Start::Continue);
        assert_eq!(plan.args, ["--continue"]);
        assert!(plan.files.is_empty() && plan.env.is_empty());
    }

    #[test]
    fn start_modes_add_their_flag_last() {
        let plain = plan_for(&on(), &LaunchOptions::default(), Start::New);
        assert!(plain.args.is_empty(), "nothing injected without MCP");
        assert!(plain.env.is_empty());
        let resume = plan_for(&on(), &LaunchOptions::default(), Start::Resume);
        assert_eq!(resume.args, ["--resume"]);
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
