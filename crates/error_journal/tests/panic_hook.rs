//! A panic on a spawned thread leaves a record in `errors.json` once the hook has run.

use std::sync::Arc;

use error_journal::{Kind, Limits, Options, Retention, SystemClock};

#[test]
fn panic_in_a_spawned_thread_is_recorded() {
    let dir = std::env::temp_dir().join(format!("error-journal-hook-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    error_journal::init(Options {
        dir: dir.clone(),
        app_version: "9.9.9".into(),
        build: "1".into(),
        retention: Retention::Week,
        clock: Arc::new(SystemClock),
        limits: Limits::default(),
    })
    .unwrap();
    error_journal::install_panic_hook();

    let joined = std::thread::Builder::new()
        .name("worker-under-test".into())
        .spawn(|| panic!("index 7 out of range for /Users/someone/file.rs"))
        .unwrap()
        .join();
    assert!(joined.is_err());

    // The hook writes synchronously: the record is on disk before `join` returns.
    let text = std::fs::read_to_string(dir.join(error_journal::FILE_NAME)).unwrap();
    let file: error_journal::JournalFile = serde_json::from_str(&text).unwrap();
    assert!(!file.clean_shutdown);
    let record = file.records.iter().find(|r| r.kind == Kind::Panic).unwrap();
    assert!(record.message.contains("index 7 out of range"));
    assert_eq!(record.thread.as_deref(), Some("worker-under-test"));
    assert!(record.location.as_deref().unwrap().contains("panic_hook.rs"));
    assert!(!record.backtrace.is_empty());
    assert_eq!(error_journal::unseen(), 1);

    error_journal::mark_all_seen();
    assert_eq!(error_journal::unseen(), 0);
    error_journal::shutdown();
    let file: error_journal::JournalFile =
        serde_json::from_str(&std::fs::read_to_string(dir.join(error_journal::FILE_NAME)).unwrap()).unwrap();
    assert!(file.clean_shutdown);
    assert!(file.records.iter().all(|r| r.seen));
    let _ = std::fs::remove_dir_all(&dir);
}
