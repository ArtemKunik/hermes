use super::*;

#[test]
fn create_in_memory_engine() {
    let engine = HermesEngine::in_memory("test-project").unwrap();
    assert_eq!(engine.project_id(), "test-project");
}

#[test]
fn search_cache_starts_empty() {
    let engine = HermesEngine::in_memory("test-cache").unwrap();
    let cache_arc = engine.search_cache();
    let guard = cache_arc.lock().unwrap();
    assert!(guard.is_empty());
}

#[test]
fn invalidate_clears_cache() {
    let engine = HermesEngine::in_memory("test-inv").unwrap();
    {
        let cache_arc = engine.search_cache();
        let mut cache = cache_arc.lock().unwrap();
        let dummy = PointerResponse::build(vec![], 0);
        cache.insert("key".to_string(), (dummy, Instant::now()));
    }
    engine.invalidate_search_cache();
    let cache_arc = engine.search_cache();
    let cache = cache_arc.lock().unwrap();
    assert!(cache.is_empty());
}

#[test]
fn busy_timeout_lets_second_connection_wait() {
    // Create a file-backed engine so two separate connections coexist.
    let temp = tempfile::NamedTempFile::new().unwrap();
    let e1 = HermesEngine::new(temp.path(), "p1").unwrap();
    let e2 = HermesEngine::new(temp.path(), "p2").unwrap();

    // Start a transaction on the first connection and hold it open for a bit.
    let mut conn1 = e1.db().lock().unwrap();
    let tx = conn1.transaction().unwrap();
    tx.execute(
        "INSERT INTO nodes (id, project_id, name, node_type) VALUES (?1,?2,?3,?4)",
        rusqlite::params!["a", "p1", "foo", "test"],
    )
    .unwrap();

    // Spawn a thread that attempts to write using the second engine.  Without a
    // busy timeout this would error immediately with "database is locked".
    let handle = std::thread::spawn(move || {
        let conn2 = e2.db().lock().unwrap();
        conn2
            .execute(
                "INSERT INTO nodes (id, project_id, name, node_type) VALUES (?1,?2,?3,?4)",
                rusqlite::params!["b", "p2", "bar", "test"],
            )
            .unwrap();
    });

    // Give the spawned thread a moment to reach the locked state.
    std::thread::sleep(Duration::from_millis(100));
    // Commit the first transaction; this should unblock the waiter.
    tx.commit().unwrap();

    // The join will panic if the inner write failed due to a busy error.
    handle.join().unwrap();
}

#[test]
fn default_busy_timeout_waits_longer_than_five_seconds() {
    std::env::remove_var("HERMES_DB_BUSY_TIMEOUT");

    let temp = tempfile::NamedTempFile::new().unwrap();
    let e1 = HermesEngine::new(temp.path(), "p1").unwrap();
    let e2 = HermesEngine::new(temp.path(), "p2").unwrap();

    let mut conn1 = e1.db().lock().unwrap();
    let tx = conn1.transaction().unwrap();
    tx.execute(
        "INSERT INTO nodes (id, project_id, name, node_type) VALUES (?1,?2,?3,?4)",
        rusqlite::params!["c", "p1", "foo", "test"],
    )
    .unwrap();

    let handle = std::thread::spawn(move || {
        let conn2 = e2.db().lock().unwrap();
        conn2
            .execute(
                "INSERT INTO nodes (id, project_id, name, node_type) VALUES (?1,?2,?3,?4)",
                rusqlite::params!["d", "p2", "bar", "test"],
            )
            .unwrap();
    });

    std::thread::sleep(Duration::from_secs(6));
    tx.commit().unwrap();
    handle.join().unwrap();
}

#[test]
fn diagnostic_busy_timeout_defaults_to_250_ms() {
    std::env::remove_var("HERMES_DIAGNOSTIC_DB_BUSY_TIMEOUT_MS");
    assert_eq!(resolve_diagnostic_busy_timeout_ms(), 250);
}

#[test]
fn diagnostic_busy_timeout_reads_env_override() {
    std::env::set_var("HERMES_DIAGNOSTIC_DB_BUSY_TIMEOUT_MS", "900");
    assert_eq!(resolve_diagnostic_busy_timeout_ms(), 900);
    std::env::remove_var("HERMES_DIAGNOSTIC_DB_BUSY_TIMEOUT_MS");
}

#[test]
fn test_search_with_goal_hint_runs() {
    let engine = HermesEngine::in_memory("test-goal").unwrap();
    let result = crate::mcp_tools::tool_search(&engine, "anything", Some("error handling"));
    assert!(result.is_ok());
}

#[test]
fn test_index_returns_busy_when_lock_exists() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("sample.rs"), "fn sample() {}").unwrap();
    std::fs::write(temp.path().join(".hermes.index.lock"), "locked").unwrap();

    let engine = HermesEngine::in_memory("test-index-busy").unwrap();
    let conn = engine.db().lock().unwrap();
    let result: serde_json::Value =
        serde_json::from_str(&crate::mcp_tools::tool_index(&engine, &conn, temp.path()).unwrap()).unwrap();

    assert_eq!(result["status"], "busy");
    assert_eq!(result["non_blocking"], true);
}
