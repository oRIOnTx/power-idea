//! Read-only MCP tools for Remote Files: list sites, list a folder, read a file. Only
//! sites with "AI read access" turned on, and only while the user has them connected:
//! an agent never opens connections, writes, deletes or runs anything on a server.

use std::sync::Arc;

use gpui_kit::*;
use remote::Connection;
use serde_json::{Value, json};

use super::{Status, Workspace};
use crate::workspace::mcp_tools::{Reply, arg_str, arg_usize};

pub(crate) const LIST_SITES: &str = "remote_list_sites";
pub(crate) const LIST_DIR: &str = "remote_list_dir";
pub(crate) const READ_FILE: &str = "remote_read_file";

/// Larger reads are cut (agents rarely need more; the file stays on the server).
const MAX_READ: usize = 512 * 1024;

impl Workspace {
    pub(in crate::workspace) fn mcp_remote_call(
        &mut self,
        name: &str,
        args: &Value,
        reply: Reply,
        cx: &mut Context<Self>,
    ) -> Option<(Reply, Result<String, String>)> {
        let Some(state) = self.remote.as_ref() else {
            return Some((reply, Err("Remote Files is not open in this window".into())));
        };
        if name == LIST_SITES {
            let sites: Vec<Value> = state
                .sites
                .iter()
                .filter(|s| s.ai_read_access)
                .map(|s| {
                    let connected = matches!(
                        state.conns.get(&s.id).map(|c| &c.status),
                        Some(Status::Connected | Status::Lost)
                    );
                    json!({ "site": s.name, "protocol": s.protocol.label(), "address": s.address(), "connected": connected })
                })
                .collect();
            return Some((reply, Ok(json!({ "sites": sites }).to_string())));
        }
        let conn = match self.mcp_remote_connection(arg_str(args, "site")) {
            Ok(conn) => conn,
            Err(err) => return Some((reply, Err(err))),
        };
        let Some(path) = arg_str(args, "path").map(|p| remote::path::normalize("/", p)) else {
            return Some((reply, Err("missing path".into())));
        };
        let tool = name.to_string();
        let limit = arg_usize(args, "max_bytes").unwrap_or(MAX_READ).min(MAX_READ);
        cx.background_spawn(async move {
            let result = match tool.as_str() {
                LIST_DIR => list_dir(&conn, &path),
                _ => read_file(&conn, &path, limit),
            };
            reply(result);
        })
        .detach();
        None
    }

    /// The live connection of an AI-readable site, or why there is none.
    fn mcp_remote_connection(&self, site: Option<&str>) -> Result<Arc<Connection>, String> {
        let name = site.ok_or("missing site")?;
        let state = self.remote.as_ref().ok_or("Remote Files is not open")?;
        let site = state
            .sites
            .iter()
            .find(|s| s.name == name || s.id == name)
            .ok_or_else(|| format!("no site named {name}"))?;
        if !site.ai_read_access {
            return Err(format!(
                "AI read access is off for {name} (Remote Files → Edit Site to allow it)"
            ));
        }
        state
            .conns
            .get(&site.id)
            .and_then(|c| c.conn.clone())
            .ok_or_else(|| format!("{name} is not connected; ask the user to connect it in Remote Files"))
    }
}

fn list_dir(conn: &Connection, path: &str) -> Result<String, String> {
    let mut entries = conn.run(|fs| fs.list(path)).map_err(|e| e.user_message())?;
    remote::listing::sort_entries(&mut entries, remote::listing::SortBy::Name);
    let rows: Vec<Value> = entries
        .iter()
        .map(|e| {
            json!({
                "name": e.name,
                "type": if e.is_dir() { "dir" } else { "file" },
                "size": e.size,
                "modified": e.modified,
                "permissions": e.permissions.map(|m| format!("{m:03o}")),
            })
        })
        .collect();
    Ok(json!({ "path": path, "entries": rows }).to_string())
}

fn read_file(conn: &Connection, path: &str, limit: usize) -> Result<String, String> {
    let entry = conn.run(|fs| fs.stat(path)).map_err(|e| e.user_message())?;
    if entry.is_dir() {
        return Err(format!("{path} is a folder; use {LIST_DIR}"));
    }
    let bytes = std::cell::RefCell::new(Vec::new());
    let read = conn.run(|fs| {
        let mut bytes = bytes.borrow_mut();
        bytes.clear();
        // Stop once past the limit: the rest is not needed.
        fs.download(path, &mut *bytes, &mut |done| done as usize <= limit)
    });
    let mut bytes = bytes.into_inner();
    match read {
        Ok(_) => {}
        Err(err) if err.kind == remote::ErrorKind::Cancelled => {}
        Err(err) => return Err(err.user_message()),
    }
    if bytes.contains(&0) {
        return Err(format!("{path} looks like a binary file"));
    }
    let truncated = bytes.len() > limit;
    bytes.truncate(limit);
    let text = String::from_utf8_lossy(&bytes).into_owned();
    Ok(json!({ "path": path, "size": entry.size, "truncated": truncated, "text": text }).to_string())
}
