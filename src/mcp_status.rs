// TRACK-041 Phase 3: MCP Lifecycle & Health Surface
//
// Exposes `hermes_mcp_status` — a structured diagnostic endpoint that returns
// indexing state, runtime configuration, tool inventory, and capability hints.

use crate::HermesEngine;
use anyhow::Result;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::sync::atomic::Ordering;

#[derive(Default)]
struct IndexStatus {
    in_progress: bool,
    lock_owner_pid: Option<u32>,
    lock_owner_alive: Option<bool>,
    lock_stale: bool,
    lock_age_secs: Option<u64>,
}

#[derive(Default)]
struct GraphCounts {
    total_nodes: i64,
    total_files: i64,
    last_index_at: Option<String>,
}

struct RuntimeStatus {
    busy_timeout: u64,
    cache_entries: usize,
    pending_calls: usize,
}

/// Return a structured health / status payload for operator diagnostics.
pub fn tool_mcp_status(engine: &HermesEngine, project_root: &std::path::Path) -> Result<Value> {
    // try_lock() instead of lock() to break the cascade-failure pattern:
    // if a timed-out leaked thread still holds read_db, fail fast rather than
    // blocking for up to 15 s and spawning another stalled thread.
    let diagnostic_db = engine.diagnostic_db()?;
    let conn = diagnostic_db
        .as_ref()
        .try_lock()
        .map_err(|_| anyhow::anyhow!("read_db busy — retry later"))?;

    // --- indexing state ---
    let index_status = inspect_index_status(project_root);
    let graph_counts = load_graph_counts(&conn, engine.project_id());
    let runtime = read_runtime_status(engine);
    let tool_names = crate::mcp_tool_schemas::tool_name_list();
    let circuit_state = engine
        .tool_circuit_breaker()
        .snapshot_for_tools(&tool_names);

    Ok(build_status_payload(
        engine,
        &index_status,
        &graph_counts,
        &runtime,
        tool_names,
        circuit_state,
    ))
}

fn build_status_payload(
    engine: &HermesEngine,
    index_status: &IndexStatus,
    graph_counts: &GraphCounts,
    runtime: &RuntimeStatus,
    tool_names: Vec<String>,
    circuit_state: Value,
) -> Value {
    json!({
        "status": "ok",
        "protocol_version": "2024-11-05",
        "server_version": env!("CARGO_PKG_VERSION"),
        "project_id": engine.project_id(),
        "session_id": engine.session_id(),
        "duplicate_process_detected": engine.duplicate_process_detected(),
        "indexing": {
            "in_progress": index_status.in_progress,
            "total_nodes": graph_counts.total_nodes,
            "total_files": graph_counts.total_files,
            "last_index_at": graph_counts.last_index_at,
            "lock_owner_pid": index_status.lock_owner_pid,
            "lock_owner_alive": index_status.lock_owner_alive,
            "lock_stale": index_status.lock_stale,
            "lock_age_secs": index_status.lock_age_secs
        },
        "runtime": {
            "db_busy_timeout_secs": runtime.busy_timeout,
            "search_cache_entries": runtime.cache_entries,
            "pending_calls": runtime.pending_calls
        },
        "tools": {
            "count": tool_names.len(),
            "list": tool_names
        },
        "circuits": circuit_state,
        "capabilities": {
            "ast_chunking": cfg!(feature = "ast"),
            "embeddings": true,
            "embedding_mode": crate::embedding::embedding_mode()
        }
    })
}

fn load_graph_counts(conn: &rusqlite::Connection, project_id: &str) -> GraphCounts {
    let total_nodes = conn
        .query_row(
            "SELECT COUNT(*) FROM nodes WHERE project_id = ?1",
            [project_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let total_files = conn
        .query_row(
            "SELECT COUNT(DISTINCT file_path) FROM nodes WHERE project_id = ?1",
            [project_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let last_index_at = conn
        .query_row(
            "SELECT MAX(created_at) FROM nodes WHERE project_id = ?1",
            [project_id],
            |r| r.get(0),
        )
        .unwrap_or(None);

    GraphCounts {
        total_nodes,
        total_files,
        last_index_at,
    }
}

fn read_cache_entries(engine: &HermesEngine) -> usize {
    engine.search_cache().lock().map(|c| c.len()).unwrap_or(0)
}

fn read_runtime_status(engine: &HermesEngine) -> RuntimeStatus {
    RuntimeStatus {
        busy_timeout: crate::resolve_busy_timeout_secs(),
        cache_entries: read_cache_entries(engine),
        pending_calls: engine.pending_calls().load(Ordering::Relaxed),
    }
}

fn inspect_index_status(project_root: &Path) -> IndexStatus {
    let lock_path = crate::index_lock::lock_file_path(project_root);
    if !lock_path.exists() {
        return IndexStatus::default();
    }

    let content = fs::read_to_string(&lock_path).unwrap_or_default();
    let owner_pid = content
        .lines()
        .find_map(|line| line.strip_prefix("pid=")?.trim().parse::<u32>().ok());
    let owner_alive = owner_pid.map(crate::index_lock::process_is_alive);
    let lock_age_secs = fs::metadata(&lock_path)
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|modified| modified.elapsed().ok())
        .map(|age| age.as_secs());
    let lock_stale = matches!(owner_alive, Some(false));

    IndexStatus {
        in_progress: !lock_stale,
        lock_owner_pid: owner_pid,
        lock_owner_alive: owner_alive,
        lock_stale,
        lock_age_secs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_status_returns_valid_json() {
        let engine = HermesEngine::in_memory("status-test").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let result = tool_mcp_status(&engine, dir.path()).unwrap();

        assert_eq!(result["status"], "ok");
        assert_eq!(result["protocol_version"], "2024-11-05");
        assert!(result["tools"]["count"].as_u64().unwrap() > 0);
        assert!(result["tools"]["list"].is_array());
        assert_eq!(result["indexing"]["in_progress"], false);
    }

    #[test]
    fn test_mcp_status_reflects_indexed_content() {
        let engine = HermesEngine::in_memory("status-idx").unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("hello.rs"), "fn main() {}").unwrap();

        // Keep this test deterministic by inserting a representative node directly
        // instead of invoking the parallel ingestion pipeline.
        {
            let conn = engine.db().lock().unwrap();
            conn.execute(
                "INSERT INTO nodes (id, project_id, name, node_type, file_path, start_line, end_line, summary) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![
                    "status-node-1",
                    engine.project_id(),
                    "hello",
                    "function",
                    dir.path().join("hello.rs").to_string_lossy().to_string(),
                    1,
                    1,
                    "fn main",
                ],
            )
            .unwrap();
        }

        let result = tool_mcp_status(&engine, dir.path()).unwrap();
        assert!(result["indexing"]["total_nodes"].as_i64().unwrap() > 0);
        assert!(result["indexing"]["total_files"].as_i64().unwrap() > 0);
    }

    #[test]
    fn test_mcp_status_detects_index_lock() {
        let engine = HermesEngine::in_memory("status-lock").unwrap();
        let dir = tempfile::tempdir().unwrap();

        let lock_path = crate::index_lock::lock_file_path(dir.path());
        std::fs::write(&lock_path, format!("pid={}", std::process::id())).unwrap();
        let result = tool_mcp_status(&engine, dir.path()).unwrap();
        assert_eq!(result["indexing"]["in_progress"], true);
        std::fs::remove_file(&lock_path).unwrap();
    }

    #[test]
    fn test_mcp_status_marks_dead_lock_owner_as_stale() {
        let engine = HermesEngine::in_memory("status-stale-lock").unwrap();
        let dir = tempfile::tempdir().unwrap();

        let lock_path = crate::index_lock::lock_file_path(dir.path());
        std::fs::write(&lock_path, "pid=999999").unwrap();

        let result = tool_mcp_status(&engine, dir.path()).unwrap();

        assert_eq!(result["indexing"]["in_progress"], false);
        assert_eq!(result["indexing"]["lock_stale"], true);
        assert_eq!(result["indexing"]["lock_owner_pid"], 999999);
        assert_eq!(result["indexing"]["lock_owner_alive"], false);
        assert!(result["indexing"]["lock_age_secs"].as_u64().is_some());
    }

    #[test]
    fn test_mcp_status_includes_circuit_state_for_tools() {
        let engine = HermesEngine::in_memory("status-circuit").unwrap();
        let dir = tempfile::tempdir().unwrap();

        let result = tool_mcp_status(&engine, dir.path()).unwrap();
        let circuits = result["circuits"].as_object().unwrap();

        assert_eq!(
            circuits.get("hermes_recall").and_then(|v| v.as_str()),
            Some("CLOSED")
        );
    }
}
