//! Live tests against throwaway servers in Docker (ignored by default):
//!
//! ```sh
//! docker run -d --rm --name pi-sftp -p 2222:22 atmoz/sftp foo:pass:::upload
//! docker run -d --rm --name pi-ftp -p 2121:21 -p 21000-21010:21000-21010 \
//!   -e USERS="foo|pass" -e ADDRESS=127.0.0.1 delfer/alpine-ftp-server
//! cargo test -p remote --test live -- --ignored --test-threads=1
//! docker stop pi-sftp pi-ftp
//! ```
//!
//! The host key goes into a temporary known_hosts, never `~/.ssh/known_hosts`.

// The helpers below are test code too: failing loudly is their job.
#![allow(clippy::expect_used, clippy::panic)]

use std::sync::atomic::AtomicBool;

use remote::transfer::{self, Direction};
use remote::{ConnectOptions, Credentials, ErrorKind, Protocol, RemoteFs, Site};

fn credentials() -> Credentials {
    Credentials {
        password: Some("pass".into()),
        passphrase: None,
    }
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("power-idea-live-{}-{name}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// Exercises every `RemoteFs` operation inside `base`.
fn round_trip(fs: &dyn RemoteFs, base: &str) {
    let dir = format!("{base}/power-idea-test");
    fs.delete(&dir).ok();
    fs.mkdir(&dir).expect("mkdir");
    fs.write(&format!("{dir}/a.txt"), b"hello").expect("write");
    assert_eq!(fs.read(&format!("{dir}/a.txt")).expect("read"), b"hello");
    let entry = fs.stat(&format!("{dir}/a.txt")).expect("stat");
    assert_eq!(entry.size, 5);
    fs.rename(&format!("{dir}/a.txt"), &format!("{dir}/b.txt"))
        .expect("rename");
    fs.chmod(&format!("{dir}/b.txt"), 0o600).expect("chmod");
    let names: Vec<String> = fs.list(&dir).expect("list").into_iter().map(|e| e.name).collect();
    assert_eq!(names, vec!["b.txt".to_string()]);
    let missing = fs.stat(&format!("{dir}/nope")).expect_err("missing file");
    assert_eq!(missing.kind, ErrorKind::NotFound);
    fs.delete(&dir).expect("delete");
    assert!(!fs.exists(&dir).expect("exists"));
}

#[test]
#[ignore = "needs atmoz/sftp on localhost:2222"]
fn sftp_host_key_flow_and_operations() {
    let dir = temp_dir("sftp");
    let known_hosts = dir.join("known_hosts");
    let mut site = Site::new("local sftp", Protocol::Sftp, "127.0.0.1", "foo");
    site.port = 2222;
    site.auth = remote::Auth::Password;
    let options = ConnectOptions {
        known_hosts: Some(known_hosts.clone()),
        trusted_fingerprint: None,
    };

    // First contact: the key is unknown and must be confirmed.
    let err = remote::connect(&site, &credentials(), &options)
        .err()
        .expect("unknown key rejected");
    let ErrorKind::HostKeyUnknown(key) = err.kind else {
        panic!("expected an unknown host key, got {err:?}");
    };
    assert!(key.fingerprint.starts_with("SHA256:"));
    remote::trust_host_key(&key, &known_hosts).expect("trust");

    let conn = remote::open(&site, credentials(), options).expect("connect after trusting");
    let home = conn.run(|fs| fs.home()).expect("home");
    conn.run(|fs| {
        round_trip(fs, &format!("{home}/upload"));
        Ok(())
    })
    .expect("round trip");

    // Folder upload and download through the transfer queue.
    let local = dir.join("site");
    std::fs::create_dir_all(local.join("css")).expect("mkdir");
    std::fs::write(local.join("css/a.css"), "a{}").expect("write");
    let plan = transfer::plan_upload(&local, &format!("{home}/upload")).expect("plan");
    transfer::execute(
        &conn,
        Direction::Upload,
        &plan,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .expect("upload");
    let entry = conn
        .run(|fs| fs.stat(&format!("{home}/upload/site")))
        .expect("stat");
    let back = dir.join("back");
    std::fs::create_dir_all(&back).expect("mkdir");
    let plan = conn
        .run(|fs| transfer::plan_download(fs, &entry, &back))
        .expect("plan");
    transfer::execute(
        &conn,
        Direction::Download,
        &plan,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .expect("download");
    assert_eq!(
        std::fs::read_to_string(back.join("site/css/a.css")).expect("read"),
        "a{}"
    );
    conn.run(|fs| fs.delete(&format!("{home}/upload/site")))
        .expect("cleanup");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
#[ignore = "needs atmoz/sftp on localhost:2222"]
fn sftp_wrong_password_is_an_auth_error() {
    let dir = temp_dir("sftp-auth");
    let mut site = Site::new("local sftp", Protocol::Sftp, "127.0.0.1", "foo");
    site.port = 2222;
    site.auth = remote::Auth::Password;
    // Trust whatever key the server has for this check.
    let probe = ConnectOptions {
        known_hosts: Some(dir.join("known_hosts")),
        trusted_fingerprint: None,
    };
    let Some(ErrorKind::HostKeyUnknown(key)) = remote::connect(&site, &credentials(), &probe)
        .err()
        .map(|e| e.kind)
    else {
        panic!("expected an unknown host key");
    };
    let options = ConnectOptions {
        trusted_fingerprint: Some(key.fingerprint),
        ..probe
    };
    let wrong = Credentials {
        password: Some("wrong".into()),
        passphrase: None,
    };
    let err = remote::connect(&site, &wrong, &options).err().expect("rejected");
    assert_eq!(err.kind, ErrorKind::Auth);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
#[ignore = "needs delfer/alpine-ftp-server on localhost:2121"]
fn ftp_operations() {
    let mut site = Site::new("local ftp", Protocol::Ftp, "127.0.0.1", "foo");
    site.port = 2121;
    let conn = remote::open(&site, credentials(), ConnectOptions::default()).expect("connect");
    let home = conn.run(|fs| fs.home()).expect("home");
    conn.run(|fs| {
        round_trip(fs, home.trim_end_matches('/'));
        Ok(())
    })
    .expect("round trip");
}
