//! Live checks against real debug adapters. Ignored by default because they
//! need the debuggers installed: `cargo test -p dap -- --ignored --nocapture`.

// Test helpers fail loudly on purpose, and print the adapter traffic for `--nocapture`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::print_stdout
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use dap::{AdapterKind, DapEvent, DebugAdapter, DebugTarget, free_port, plan};
use serde_json::{Value, json};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("power-idea-dap-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

/// Runs a debug session to the breakpoint at `line` of `source` and returns
/// (stopped line, local variables as name → value).
fn hit_breakpoint(
    target: DebugTarget,
    source: &Path,
    line: u32,
    lldb_dap: &[String],
) -> (u32, BTreeMap<String, String>) {
    let port = free_port().unwrap();
    let plan = plan(&target, port, lldb_dap).unwrap();
    let (tx, rx) = mpsc::channel();
    let adapter = DebugAdapter::start(
        &plan.adapter,
        Arc::new(move |e| {
            let _ = tx.send(e);
        }),
    )
    .expect("start adapter");
    let wait_for = |name: &str| -> Value {
        loop {
            match rx.recv_timeout(Duration::from_secs(30)).expect("event") {
                DapEvent::Event { event, body } if event == name => return body,
                DapEvent::Event { event, body } if event == "stopped" => panic!("unexpected stop: {body}"),
                DapEvent::Closed => panic!("adapter closed while waiting for {name}"),
                other => println!("  {other:?}"),
            }
        }
    };

    let caps = adapter
        .request(
            "initialize",
            json!({ "clientID": "power-idea", "adapterID": target.kind.label(), "linesStartAt1": true,
                    "columnsStartAt1": true, "pathFormat": "path" }),
        )
        .unwrap();
    println!("capabilities: {caps}");
    // launch/attach may only answer after configurationDone.
    let launcher = {
        let adapter = adapter.clone();
        let (request, arguments) = (plan.request, plan.arguments.clone());
        std::thread::spawn(move || adapter.request(request, arguments))
    };
    wait_for("initialized");
    let set = adapter
        .request(
            "setBreakpoints",
            json!({ "source": { "path": source }, "breakpoints": [{ "line": line }] }),
        )
        .unwrap();
    println!("breakpoints: {set}");
    adapter.request("configurationDone", json!({})).unwrap();

    let mut stopped = wait_for("stopped");
    if target.kind.stops_at_entry() {
        let thread = stopped["threadId"].clone();
        adapter
            .request("continue", json!({ "threadId": thread }))
            .unwrap();
        stopped = wait_for("stopped");
    }
    let thread = stopped["threadId"].clone();
    let trace = adapter
        .request("stackTrace", json!({ "threadId": thread, "levels": 20 }))
        .unwrap();
    let frame = &trace["stackFrames"][0];
    let scopes = adapter
        .request("scopes", json!({ "frameId": frame["id"] }))
        .unwrap();
    let reference = scopes["scopes"][0]["variablesReference"].clone();
    let variables = adapter
        .request("variables", json!({ "variablesReference": reference }))
        .unwrap();
    let locals = variables["variables"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            (
                v["name"].as_str().unwrap_or("").to_string(),
                v["value"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect();
    let _ = adapter.request("continue", json!({ "threadId": thread }));
    let _ = launcher.join();
    adapter.kill();
    (frame["line"].as_u64().unwrap() as u32, locals)
}

#[test]
#[ignore = "needs the rdbg Ruby debugger installed"]
fn rdbg_stops_at_a_ruby_breakpoint() {
    let dir = temp_dir("ruby");
    let script = dir.join("app.rb");
    std::fs::write(&script, "x = 41\ny = x + 1\nputs y\n").unwrap();
    let target = DebugTarget {
        kind: AdapterKind::Ruby,
        command: "ruby app.rb".into(),
        program: None,
        module: None,
        args: Vec::new(),
        cwd: dir,
        env: BTreeMap::new(),
        shell: std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into()),
    };
    let (line, locals) = hit_breakpoint(target, &script, 2, &[]);
    println!("locals: {locals:?}");
    assert_eq!(line, 2);
    assert_eq!(locals.get("x").map(String::as_str), Some("41"));
}

#[test]
#[ignore = "needs Xcode command line tools (cc, lldb-dap)"]
fn lldb_dap_stops_at_a_c_breakpoint() {
    let dir = temp_dir("lldb");
    let source = dir.join("main.c");
    std::fs::write(
        &source,
        "int main(void) {\n  int x = 41;\n  int y = x + 1;\n  return y - 42;\n}\n",
    )
    .unwrap();
    let built = std::process::Command::new("cc")
        .args(["-g", "-O0", "-o", "app", "main.c"])
        .current_dir(&dir)
        .status()
        .unwrap();
    assert!(built.success());
    let target = DebugTarget {
        kind: AdapterKind::Lldb,
        command: String::new(),
        program: Some("app".into()),
        module: None,
        args: Vec::new(),
        cwd: dir,
        env: BTreeMap::new(),
        shell: "/bin/zsh".into(),
    };
    let (line, locals) = hit_breakpoint(target, &source, 3, &["xcrun".into(), "lldb-dap".into()]);
    println!("locals: {locals:?}");
    assert_eq!(line, 3);
    assert_eq!(locals.get("x").map(String::as_str), Some("41"));
}
