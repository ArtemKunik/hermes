// tools/hermes-engine/src/mcp_tools.rs
//
// MCP tool implementations, extracted from mcp_server.rs for size compliance.
// Each function returns a pretty-printed JSON string for the MCP text response.
//
// Validation tools live in mcp_tools_validation.rs.
// Stats/fact tools live in mcp_tools_stats.rs.
// Graph-traversal tools (repo_map, impact_analysis) live in mcp_tools_graph.rs.
// Duplicate-detection / search-miss tools live in mcp_tools_analysis.rs.

use anyhow::Result;
use rusqlite::Connection;
use serde_json::json;
use std::path::Path;
use std::time::Duration;

use crate::{
    accounting::Accountant,
    graph::KnowledgeGraph,
    ingestion::{crawler, IngestionPipeline},
    search::{SearchEngine, SearchMode},
    HermesEngine,
};

// Re-export tools defined in sub-modules so callers use `mcp_tools::tool_*`.
pub use crate::mcp_tools_analysis::{tool_scan_duplicates, tool_search_misses};
pub use crate::mcp_tools_graph::{tool_impact_analysis, tool_repo_map};
pub use crate::mcp_tools_stats::{
    tool_add_fact,
    tool_add_fact_with_conn,
    tool_list_facts,
    tool_stats,
    tool_stats_with_conn,
};

pub fn tool_search(engine: &HermesEngine, query: &str, goal: Option<&str>) -> Result<String> {
    // Goal-hint: when provided, prefix the query to bias semantic/FTS results
    // toward the agent's current information need (SWE-Pruner concept).
    let effective_query = match goal {
        Some(g) if !g.is_empty() => format!("{g}: {query}"),
        _ => query.to_string(),
    };
    // Use the read connection so FTS search doesn't contend with auto-index
    // writes on the main db mutex.
    let graph = KnowledgeGraph::new(engine.read_db().clone(), engine.project_id());
    let search = SearchEngine::new(&graph, engine.search_cache());
    let resp = search.search(&effective_query, 10, &SearchMode::Smart)?;

    // Count how many pointers came from memory/ paths
    let memory_hits = resp
        .pointers
        .iter()
        .filter(|p| crawler::is_memory_path(&p.source))
        .count() as u64;

    let acct = Accountant::new(
        engine.db().clone(),
        engine.project_id(),
        engine.session_id(),
    );
    acct.record_query_with_memory(
        query,
        resp.accounting.pointer_tokens,
        0,
        resp.accounting.traditional_rag_estimate,
        memory_hits,
    )?;

    Ok(serde_json::to_string_pretty(&resp)?)
}

pub fn tool_fetch(engine: &HermesEngine, node_id: &str) -> Result<String> {
    let graph = KnowledgeGraph::new(engine.read_db().clone(), engine.project_id());
    let search = SearchEngine::new(&graph, engine.search_cache());
    let Some(resp) = search.fetch(node_id)? else {
        anyhow::bail!("node not found: {node_id}");
    };
    let acct = Accountant::new(
        engine.db().clone(),
        engine.project_id(),
        engine.session_id(),
    );
    let traditional = if std::env::var("HERMES_ENABLE_REAL_TRADITIONAL_ESTIMATE")
        .map(|v| v != "0" && v.to_lowercase() != "false")
        .unwrap_or(false)
    {
        resp.content_tokens.unwrap_or(resp.token_count)
    } else {
        resp.token_count.saturating_mul(15)
    };
    acct.record_query(node_id, 0, resp.token_count, traditional)?;
    Ok(serde_json::to_string_pretty(&resp)?)
}

pub fn tool_index(engine: &HermesEngine, conn: &Connection, project_root: &Path) -> Result<String> {
    let _index_lock = match crate::index_lock::try_acquire_index_lock(project_root)? {
        crate::index_lock::LockAcquisition::Acquired(lock) => lock,
        crate::index_lock::LockAcquisition::Busy(_) => {
            eprintln!(
                "[hermes:index] status=busy project_root={} current_pid={}",
                project_root.display(),
                std::process::id()
            );
            return Ok(serde_json::to_string_pretty(&json!({
                "status": "busy",
                "reason": "index already running",
                "non_blocking": true,
            }))?);
        }
    };

    eprintln!(
        "[hermes:index] status=started project_root={} current_pid={}",
        project_root.display(),
        std::process::id()
    );



    let graph = KnowledgeGraph::from_conn(conn, engine.project_id());
    let pipeline = IngestionPipeline::new(&graph);
    let report = pipeline.ingest_directory(project_root)?;
    engine.invalidate_search_cache();
    eprintln!(
        "[hermes:index] status=completed project_root={} current_pid={} total_files={} indexed={} skipped={} errors={} nodes_created={}",
        project_root.display(),
        std::process::id(),
        report.total_files,
        report.indexed,
        report.skipped,
        report.errors,
        report.nodes_created
    );
    Ok(serde_json::to_string_pretty(&json!({
        "total_files": report.total_files, "indexed": report.indexed,
        "skipped": report.skipped, "errors": report.errors,
        "nodes_created": report.nodes_created,
    }))?)
}

pub fn tool_backfill(engine: &HermesEngine) -> Result<String> {
    let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
    let pipeline = IngestionPipeline::new(&graph);
    let updated = pipeline.backfill_content_tokens()?;
    engine.invalidate_search_cache();
    Ok(serde_json::to_string_pretty(
        &json!({ "nodes_updated": updated }),
    )?)
}

/// Build a blocking HTTP client with a conservative timeout so that actor
/// threads calling unreachable Mastermind endpoints do not leak indefinitely.
/// Without this, every hermes tool that proxies to Mastermind could block
/// for the OS TCP timeout (minutes), saturating the actor thread pool and
/// causing unrelated tools (hermes_mcp_status, hermes_stats) to time out.
fn mastermind_client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_default()
}

/// Generic Mastermind proxy — handles URL resolution, client construction, and response deserialization for POST requests.
pub fn proxy_to_mastermind(engine: &HermesEngine, path_suffix: &str) -> Result<String> {
    let mastermind_url = std::env::var("MASTERMIND_API_URL")
        .unwrap_or_else(|_| "http://localhost:8080".to_string());

    let client = mastermind_client();
    let resp = client.post(format!("{mastermind_url}{path_suffix}")).send()?;
    let body: serde_json::Value = resp.json()?;
    Ok(serde_json::to_string_pretty(&body)?)
}

pub fn tool_slow_loop_status(_engine: &HermesEngine) -> Result<String> {
    let mastermind_url = std::env::var("MASTERMIND_API_URL")
        .unwrap_or_else(|_| "http://localhost:8080".to_string());

    let client = mastermind_client();
    let resp = client
        .get(format!("{}/consolidator/status", mastermind_url))
        .send();

    match resp {
        Ok(r) if r.status().is_success() => {
            let body: serde_json::Value = r.json()?;
            Ok(serde_json::to_string_pretty(&body)?)
        }
        _ => Ok(serde_json::to_string_pretty(&json!({
            "status": "unknown",
            "reason": "mastermind-daemon unreachable or status endpoint missing"
        }))?),
    }
}

pub fn tool_generate_digest(_engine: &HermesEngine, date: &str) -> Result<String> {
    proxy_to_mastermind(_engine, &format!("/consolidator/daily-digest/{date}"))
}

pub fn tool_compact_sessions(engine: &HermesEngine) -> Result<String> {
    proxy_to_mastermind(engine, "/consolidator/compact-sessions")
}

pub fn tool_generate_weekly_brief(engine: &HermesEngine) -> Result<String> {
    proxy_to_mastermind(engine, "/consolidator/weekly-brief")
}

pub fn tool_approve_skill_candidate(_engine: &HermesEngine, name: &str) -> Result<String> {
    proxy_to_mastermind(_engine, &format!("/consolidator/approve-skill/{name}"))
}

pub fn tool_reject_skill_candidate(_engine: &HermesEngine, name: &str) -> Result<String> {
    proxy_to_mastermind(_engine, &format!("/consolidator/reject-skill/{name}"))
}

pub fn tool_apply_proposal(_engine: &HermesEngine, filename: &str) -> Result<String> {
    proxy_to_mastermind(_engine, &format!("/consolidator/apply-proposal/{filename}"))
}
