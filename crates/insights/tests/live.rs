//! Indexes the real `~/.claude` into a temp database and prints aggregates
//! only (never transcript text). Ignored: `cargo test -p insights -- --ignored --nocapture`.

#[test]
#[ignore = "reads the real ~/.claude transcripts of this machine"]
fn indexes_local_transcripts() {
    let home = std::path::PathBuf::from(std::env::var("HOME").unwrap());
    let db = std::env::temp_dir().join(format!("pi-insights-live-{}.sqlite", std::process::id()));
    let store = insights::Store::open(&db).unwrap();
    let started = std::time::Instant::now();
    let n = insights::index(&store, &home.join(".claude"), &insights::DEFAULT_WEIGHTS).unwrap();
    let sessions = store.sessions().unwrap();
    println!(
        "indexed {n} files → {} sessions in {:?}",
        sessions.len(),
        started.elapsed()
    );
    for s in sessions.iter().rev().take(5) {
        let m = &s.metrics;
        println!(
            "score {:>3.0} prompts {:>3} corr {:>3.0}% tools {:>4} err {:>3.0}% tokens {:>6.1}M ${:.2}",
            s.score,
            m.prompts,
            m.correction_rate * 100.0,
            m.tool_calls,
            m.error_rate * 100.0,
            m.total_tokens as f64 / 1e6,
            m.cost_usd
        );
    }
    let again = std::time::Instant::now();
    assert_eq!(
        insights::index(&store, &home.join(".claude"), &insights::DEFAULT_WEIGHTS).unwrap(),
        0
    );
    println!("re-index of unchanged files: {:?}", again.elapsed());
    let _ = std::fs::remove_file(&db);
}
