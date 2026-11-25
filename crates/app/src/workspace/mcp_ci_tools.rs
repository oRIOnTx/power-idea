//! MCP tools for GitHub Actions (read-only, `ci::tools`): runs, one run, failed logs. They
//! never start, re-run or cancel anything. Requests run in the background.

use gpui_kit::*;
use serde_json::Value;

use super::Workspace;
use super::mcp_tools::{Reply, arg_str, arg_usize};
use crate::github_actions::fetch;

impl Workspace {
    /// Replies from a background task (the GitHub API is slow); always `None`.
    pub(super) fn mcp_ci_call(
        &mut self,
        name: &str,
        args: &Value,
        reply: Reply,
        cx: &mut Context<Self>,
    ) -> Option<(Reply, Result<String, String>)> {
        let tool = name.to_string();
        let branch = arg_str(args, "branch").map(str::to_string);
        let limit = arg_usize(args, "limit").unwrap_or(10);
        let run_id = args.get("run_id").and_then(Value::as_u64);
        let context_limit = self.project.ci.claude_context_chars();
        // Reuse the tab's client (its ETag cache) when the tab is open.
        let client = self
            .ci
            .as_ref()
            .and_then(|handle| handle.view.read(cx).client().cloned());
        let root = self.root.clone();
        cx.background_spawn(async move {
            let client = match client.map_or_else(|| fetch::connect(&root), Ok) {
                Ok(client) => client,
                Err(message) => return reply(Err(message)),
            };
            let result = match (tool.as_str(), run_id) {
                (ci::tools::LIST_RUNS, _) => ci::tools::list_runs(&client, branch.as_deref(), limit),
                (_, None) => Err("missing run_id".into()),
                (ci::tools::GET_RUN, Some(id)) => ci::tools::get_run(&client, id),
                (_, Some(id)) => ci::tools::failed_logs(&client, id, context_limit),
            };
            reply(result);
        })
        .detach();
        None
    }
}
