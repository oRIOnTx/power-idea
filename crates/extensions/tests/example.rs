//! Runs the example extension (`examples/extensions/hello-world`, Python) end to end:
//! discovery, `initialize`, a command and the requests it makes back to the IDE.

use std::path::Path;
use std::process::Command;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use extensions::{Host, HostRequest, Incoming};
use serde_json::{Value, json};

#[test]
fn hello_world_greets_through_the_ide() {
    if Command::new("python3").arg("--version").output().is_err() {
        eprintln!("python3 not found; skipping");
        return;
    }
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/extensions");
    let found = extensions::discover(&examples);
    assert!(found.errors.is_empty(), "{:?}", found.errors);
    let extension = found
        .extensions
        .iter()
        .find(|e| e.id() == "hello-world")
        .expect("the example is discovered");
    assert!(extension.starts_eagerly());

    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    let handler: extensions::Handler = Arc::new(move |message| {
        let _ = tx.lock().unwrap().send(message);
    });
    let host = Host::start(extension, Path::new("/tmp/demo"), None, handler).unwrap();

    let command = {
        let host = host.clone();
        thread::spawn(move || host.execute("hello-world.greet"))
    };
    let mut seen = Vec::new();
    while seen.len() < 2 {
        let message = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("a request from the extension");
        let Incoming::Request { id, method, params } = message else {
            panic!("unexpected {message:?}");
        };
        let request = HostRequest::parse(&method, params).unwrap();
        let result = match &request {
            HostRequest::WorkspaceInfo {} => json!({ "name": "demo", "openFiles": ["a.rs", "b.rs"] }),
            _ => Value::Null,
        };
        host.respond(id, Ok(result));
        seen.push(request);
    }
    assert_eq!(command.join().unwrap().unwrap(), Value::Null);
    assert_eq!(seen[0], HostRequest::WorkspaceInfo {});
    let HostRequest::Notify { message, .. } = &seen[1] else {
        panic!("{:?}", seen[1]);
    };
    assert_eq!(message, "Hello from demo! 2 file(s) open.");

    let unknown = host.execute("hello-world.nope").unwrap_err();
    assert!(format!("{unknown:#}").contains("unknown command"), "{unknown:#}");
}
