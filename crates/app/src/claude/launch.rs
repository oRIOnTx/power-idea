//! The launch of `claude` for a project, shared by the Claude terminal tabs.

use std::path::{Path, PathBuf};

use claude_integration::launcher::{self, LaunchOptions, LaunchPlan, McpEndpoint, Start};
use gpui_kit::*;

use super::{Binary, ClaudeState, cache_root};
use crate::mcp_bridge::McpStatus;

/// The launch plan for the project at `root`, with its generated files in the cache folder.
pub fn plan_for(root: &Path, start: Start, cx: &App) -> Option<LaunchPlan> {
    let state = cx.try_global::<ClaudeState>()?;
    let settings = &state.settings;
    let program = match &state.binary {
        Binary::Found { path, .. } => path.clone(),
        _ => PathBuf::from("claude"),
    };
    let cache = cache_root()?.join(launcher::cache_key(root));
    let home = dirs::home_dir().unwrap_or_default();
    let mcp = settings.general.ide_mcp.then(|| mcp_endpoint(cx)).flatten();
    let options = LaunchOptions {
        ide_exe: std::env::current_exe().ok(),
        socket: Some(super::bridge_socket()),
        tag: root.to_string_lossy().into_owned(),
        mcp,
        user_statusline: launcher::user_statusline(root, &home),
    };
    Some(launcher::plan(settings, &program, root, &cache, start, &options))
}

fn mcp_endpoint(cx: &App) -> Option<McpEndpoint> {
    let port = cx.try_global::<McpStatus>()?.port?;
    let token = crate::mcp_bridge::load_or_create_token().ok()?;
    Some(McpEndpoint { port, token })
}
