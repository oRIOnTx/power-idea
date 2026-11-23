//! How to start each language's debug adapter and what to ask it to debug.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Result, bail};
use serde_json::{Value, json};

use super::{AdapterCommand, Transport};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdapterKind {
    /// Ruby's `debug` gem (rdbg), over TCP.
    Ruby,
    /// LLVM's lldb-dap: Rust, C, C++, Swift.
    Lldb,
    /// debugpy.
    Python,
    /// Delve (`dlv dap`), over TCP.
    Go,
}

impl AdapterKind {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name.to_ascii_lowercase().as_str() {
            "ruby" | "rdbg" => Self::Ruby,
            "lldb" | "rust" | "c" | "cpp" | "c++" | "swift" => Self::Lldb,
            "python" | "debugpy" => Self::Python,
            "go" | "dlv" | "delve" => Self::Go,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Ruby => "rdbg",
            Self::Lldb => "lldb-dap",
            Self::Python => "debugpy",
            Self::Go => "dlv",
        }
    }

    /// How to install the adapter when it is missing.
    pub fn install_hint(self) -> &'static str {
        match self {
            Self::Ruby => "gem install debug (or add `debug` to the Gemfile)",
            Self::Lldb => "xcode-select --install (lldb-dap ships with the Command Line Tools)",
            Self::Python => "pip install debugpy",
            Self::Go => "go install github.com/go-delve/delve/cmd/dlv@latest",
        }
    }

    /// rdbg stops at the start of the program; the session continues past that stop.
    pub fn stops_at_entry(self) -> bool {
        self == Self::Ruby
    }
}

/// What to debug.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugTarget {
    pub kind: AdapterKind,
    /// Shell command line (Ruby: what rdbg runs, e.g. `bundle exec rspec`).
    pub command: String,
    /// Program, binary or script (lldb, debugpy, dlv); relative to `cwd`.
    pub program: Option<String>,
    /// Python module to run instead of a program (`pytest`).
    pub module: Option<String>,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
    /// Login shell used to start commands (so rbenv, asdf… are set up).
    pub shell: String,
}

/// The adapter process and the `launch` / `attach` request for a target.
#[derive(Debug, Clone, PartialEq)]
pub struct LaunchPlan {
    pub adapter: AdapterCommand,
    pub request: &'static str,
    pub arguments: Value,
}

/// `lldb_dap` is the command that starts lldb-dap (e.g. `["xcrun", "lldb-dap"]`).
pub fn plan(target: &DebugTarget, port: u16, lldb_dap: &[String]) -> Result<LaunchPlan> {
    let adapter = |program: String, args: Vec<String>, transport| AdapterCommand {
        program,
        args,
        cwd: target.cwd.clone(),
        env: target.env.clone(),
        transport,
    };
    let program_path = || -> Result<String> {
        let Some(program) = target.program.as_deref().filter(|p| !p.is_empty()) else {
            bail!("set `program` in the debug section of the run configuration");
        };
        Ok(target.cwd.join(program).display().to_string())
    };
    Ok(match target.kind {
        AdapterKind::Ruby => {
            if target.command.trim().is_empty() {
                bail!("the run configuration has no command to debug");
            }
            let script = format!("exec rdbg --open --port {port} --command -- {}", target.command);
            LaunchPlan {
                adapter: adapter(
                    target.shell.clone(),
                    vec!["-l".into(), "-c".into(), script],
                    Transport::Tcp { port },
                ),
                request: "attach",
                arguments: json!({ "localfs": true }),
            }
        }
        AdapterKind::Lldb => {
            let Some((program, args)) = lldb_dap.split_first() else {
                bail!("lldb-dap was not found");
            };
            let env: Vec<String> = target.env.iter().map(|(k, v)| format!("{k}={v}")).collect();
            LaunchPlan {
                adapter: adapter(program.clone(), args.to_vec(), Transport::Stdio),
                request: "launch",
                arguments: json!({
                    "program": program_path()?,
                    "args": target.args,
                    "cwd": target.cwd,
                    "env": env,
                    "stopOnEntry": false,
                }),
            }
        }
        AdapterKind::Python => {
            // debugpy runs either a module (`-m`) or a script.
            let (entry_key, entry) = match target.module.as_deref().filter(|m| !m.is_empty()) {
                Some(module) => ("module", json!(module)),
                None => ("program", json!(program_path()?)),
            };
            let arguments = json!({
                "args": target.args,
                "cwd": target.cwd,
                "env": target.env,
                "console": "internalConsole",
                "justMyCode": true,
                entry_key: entry,
            });
            LaunchPlan {
                adapter: adapter(
                    "python3".into(),
                    vec!["-m".into(), "debugpy.adapter".into()],
                    Transport::Stdio,
                ),
                request: "launch",
                arguments,
            }
        }
        AdapterKind::Go => LaunchPlan {
            adapter: adapter(
                "dlv".into(),
                vec!["dap".into(), "--listen".into(), format!("127.0.0.1:{port}")],
                Transport::Tcp { port },
            ),
            request: "launch",
            arguments: json!({
                "mode": "debug",
                "program": target.cwd.join(target.program.as_deref().unwrap_or(".")),
                "args": target.args,
                "cwd": target.cwd,
                "env": target.env,
            }),
        },
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use super::{AdapterKind, DebugTarget, plan};
    use crate::Transport;

    fn target(kind: AdapterKind) -> DebugTarget {
        DebugTarget {
            kind,
            command: "bundle exec rspec".into(),
            program: Some("target/debug/app".into()),
            module: None,
            args: vec!["--verbose".into()],
            cwd: PathBuf::from("/p"),
            env: BTreeMap::from([("RUST_LOG".into(), "debug".into())]),
            shell: "/bin/zsh".into(),
        }
    }

    #[test]
    fn plans_each_adapter() {
        let ruby = plan(&target(AdapterKind::Ruby), 4000, &[]).unwrap();
        assert_eq!(ruby.adapter.transport, Transport::Tcp { port: 4000 });
        assert_eq!(
            ruby.adapter.args[2],
            "exec rdbg --open --port 4000 --command -- bundle exec rspec"
        );
        assert_eq!(ruby.request, "attach");

        let lldb = plan(
            &target(AdapterKind::Lldb),
            0,
            &["xcrun".into(), "lldb-dap".into()],
        )
        .unwrap();
        assert_eq!(
            (lldb.adapter.program.as_str(), lldb.adapter.args.as_slice()),
            ("xcrun", &["lldb-dap".to_string()][..])
        );
        assert_eq!(lldb.arguments["program"], "/p/target/debug/app");
        assert_eq!(lldb.arguments["env"][0], "RUST_LOG=debug");
        assert!(plan(&target(AdapterKind::Lldb), 0, &[]).is_err());

        let python = plan(
            &DebugTarget {
                module: Some("pytest".into()),
                ..target(AdapterKind::Python)
            },
            0,
            &[],
        )
        .unwrap();
        assert_eq!(python.arguments["module"], "pytest");
        assert!(python.arguments.get("program").is_none());

        let go = plan(
            &DebugTarget {
                program: None,
                ..target(AdapterKind::Go)
            },
            5000,
            &[],
        )
        .unwrap();
        assert_eq!(go.adapter.args, ["dap", "--listen", "127.0.0.1:5000"]);
        assert_eq!(go.arguments["program"], "/p/.");
    }

    #[test]
    fn parses_adapter_names() {
        assert_eq!(AdapterKind::parse("Rust"), Some(AdapterKind::Lldb));
        assert_eq!(AdapterKind::parse("rdbg"), Some(AdapterKind::Ruby));
        assert_eq!(AdapterKind::parse("cobol"), None);
    }
}
