use anyhow::Result;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

use crate::{
    accounting::Accountant,
    engine_cache::EngineCache,
    graph::KnowledgeGraph,
    ingestion::IngestionPipeline,
    lock_ext::LockExt,
    mcp_tools_validation::{tool_check_consistency, tool_validate_env},
    search::{SearchEngine, SearchMode},
    temporal::{FactType, TemporalStore},
    HermesEngine,
};

pub(crate) fn dispatch(cache: &EngineCache, method: &str, params: &Value) -> Result<Value> {
    match method {
        "initialize" => Ok(handle_initialize(cache)),
        "tools/list" => Ok(crate::mcp_tools_schema::handle_tools_list()),
        "tools/call" => handle_tool_call(cache, params),
        other => anyhow::bail!("unknown method: {other}"),
    }
}

fn handle_initialize(cache: &EngineCache) -> Value {
    json!({
        "protocolVersion": "2024-11-05",
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": {
            "name": "Hermes",
            "version": env!("CARGO_PKG_VERSION"),
            "project_root": cache.default_root.display().to_string(),
            "project_id":   cache.default_engine.project_id(),
            "projects":     cache.list_projects(),
            "hint": "Use project_id (short name) or full project_root path as the project_root argument. Call hermes_list_projects to see all available projects."
        }
    })
}

fn handle_tool_call(cache: &EngineCache, params: &Value) -> Result<Value> {
    let name = params["name"].as_str().unwrap_or("");
    let args = &params["arguments"];

    // 1. Special cases that never need project_root
    if name == "hermes_list_projects" {
        let text = tool_list_projects(cache)?;
        return Ok(json!({ "content": [{"type": "text", "text": text }] }));
    }
    if name == "hermes_mcp_status" {
        let text = tool_mcp_status(&cache.default_engine)?;
        return Ok(json!({ "content": [{ "type": "text", "text": text }] }));
    }

    // 2. Resolve engine (use provided project_root or fallback to default)
    let project_root_arg = args["project_root"].as_str().unwrap_or("");
    let (engine, project_root) = if project_root_arg.is_empty() {
        (cache.default_engine.clone(), cache.default_root.clone())
    } else {
        cache.resolve(Some(project_root_arg))?
    };

    info!(
        "[hermes] tool={} project_id={} project_root={}",
        name,
        engine.project_id(),
        project_root.display()
    );

    let text = match name {
        "hermes_search" => {
            let query = args["query"].as_str().unwrap_or("");
            anyhow::ensure!(!query.is_empty(), "hermes_search requires 'query'");
            tool_search(&engine, query, &project_root, project_root_arg)?
        }
        "hermes_fetch" => {
            let node_id = args["node_id"].as_str().unwrap_or("");
            anyhow::ensure!(!node_id.is_empty(), "hermes_fetch requires 'node_id'");
            tool_fetch(&engine, node_id, &project_root)?
        }
        "hermes_index" => {
            let force = args["force"].as_bool().unwrap_or(false);
            tool_index(&engine, &project_root, force)?
        }
        "hermes_stats" => tool_stats(&engine)?,
        "hermes_fact" => {
            let ft = args["fact_type"].as_str().unwrap_or("");
            let c = args["content"].as_str().unwrap_or("");
            anyhow::ensure!(
                !ft.is_empty() && !c.is_empty(),
                "hermes_fact requires 'fact_type' and 'content'"
            );
            tool_add_fact(&engine, ft, c)?
        }
        "hermes_facts" => {
            let filter = args["fact_type"].as_str();
            tool_list_facts(&engine, filter)?
        }
        "hermes_validate_env" => {
            let var = args["env_var"].as_str().unwrap_or("");
            anyhow::ensure!(!var.is_empty(), "hermes_validate_env requires 'env_var'");
            tool_validate_env(&engine, var)?
        }
        "hermes_check_consistency" => tool_check_consistency(&engine)?,
        other => anyhow::bail!("unknown tool: {other}"),
    };

    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
}

fn tool_list_projects(cache: &EngineCache) -> Result<String> {
    Ok(serde_json::to_string_pretty(&json!({
        "projects": cache.list_projects(),
        "usage": "Pass project_id or project_root to any hermes tool to target that project."
    }))?)
}

fn tool_search(engine: &HermesEngine, query: &str, project_root: &Path, requested: &str) -> Result<String> {
    let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
    let search = SearchEngine::new(&graph, engine.search_cache());
    let resp = search.search(query, 10, &SearchMode::Smart)?;
    let acct = Accountant::new(
        engine.db().clone(),
        engine.project_id(),
        engine.session_id(),
    );
    acct.record_query(
        query,
        resp.accounting.pointer_tokens,
        0,
        resp.accounting.traditional_rag_estimate,
    )?;
    let mut out = serde_json::to_value(&resp)?;
    out["project_id"] = json!(engine.project_id());
    out["project_root"] = json!(project_root.display().to_string());
    // Warn when the requested arg was a full path that resolved to a different canonical location.
    // (Name-based lookups intentionally resolve to a different path and are not warned.)
    let resolved_canonical = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    let requested_as_path = PathBuf::from(requested);
    let looks_like_path = requested_as_path.is_absolute()
        || requested.contains('/')
        || requested.contains('\\');
    if looks_like_path {
        let requested_canonical = requested_as_path
            .canonicalize()
            .unwrap_or_else(|_| requested_as_path.clone());
        if requested_canonical != resolved_canonical {
            let warning = format!(
                "Searched project '{}' at '{}' but you passed '{}'. \
                 Call hermes_list_projects to see correct project_root values.",
                engine.project_id(),
                project_root.display(),
                requested
            );
            out["WARNING"] = json!(warning);
            warn!("[hermes] WARNING: {}", warning);
        }
    }
    Ok(serde_json::to_string_pretty(&out)?)
}

fn tool_fetch(engine: &HermesEngine, node_id: &str, project_root: &Path) -> Result<String> {
    let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
    let search = SearchEngine::new(&graph, engine.search_cache());
    let Some(resp) = search.fetch(node_id)? else {
        anyhow::bail!("node not found: {node_id}");
    };
    let acct = Accountant::new(
        engine.db().clone(),
        engine.project_id(),
        engine.session_id(),
    );
    acct.record_query(node_id, 0, resp.token_count, resp.token_count.saturating_mul(15))?;
    let mut out = serde_json::to_value(&resp)?;
    out["project_id"] = json!(engine.project_id());
    out["project_root"] = json!(project_root.display().to_string());
    Ok(serde_json::to_string_pretty(&out)?)
}

fn tool_index(engine: &HermesEngine, project_root: &Path, force: bool) -> Result<String> {
    let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
    let pipeline = IngestionPipeline::new(&graph);
    let report = if force {
        pipeline.ingest_directory_force(project_root)?
    } else {
        pipeline.ingest_directory(project_root)?
    };
    engine.invalidate_search_cache();
    Ok(serde_json::to_string_pretty(&json!({
        "project_id": engine.project_id(),
        "project_root": project_root.display().to_string(),
        "total_files": report.total_files, "indexed": report.indexed,
        "skipped": report.skipped, "errors": report.errors,
        "nodes_created": report.nodes_created,
    }))?)
}

fn tool_stats(engine: &HermesEngine) -> Result<String> {
    let acct = Accountant::new(
        engine.db().clone(),
        engine.project_id(),
        engine.session_id(),
    );
    let today = acct.get_today_stats()?;
    let cumulative = acct.get_cumulative_stats()?;
    Ok(serde_json::to_string_pretty(&json!({
        "today": {
            "total_queries":            today.total_queries,
            "pointer_tokens_used":      today.total_pointer_tokens,
            "fetched_tokens_used":      today.total_fetched_tokens,
            "traditional_rag_estimate": today.total_traditional_estimate,
            "tokens_saved":             today.cumulative_savings_tokens,
            "savings_pct":              format!("{:.1}%", today.cumulative_savings_pct),
        },
        "cumulative": {
            "total_queries":            cumulative.total_queries,
            "pointer_tokens_used":      cumulative.total_pointer_tokens,
            "fetched_tokens_used":      cumulative.total_fetched_tokens,
            "traditional_rag_estimate": cumulative.total_traditional_estimate,
            "tokens_saved":             cumulative.cumulative_savings_tokens,
            "savings_pct":              format!("{:.1}%", cumulative.cumulative_savings_pct),
        },
    }))?)
}

fn tool_mcp_status(engine: &HermesEngine) -> Result<String> {
    let db = engine.db().lock_ctx("tool_mcp_status")?;
    let total_nodes: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM nodes WHERE project_id = ?1",
            rusqlite::params![engine.project_id()],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let total_files: i64 = db
        .query_row(
            "SELECT COUNT(DISTINCT file_path) FROM nodes WHERE project_id = ?1 AND file_path IS NOT NULL",
            rusqlite::params![engine.project_id()],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let has_vectors: bool = db
        .query_row(
            "SELECT COUNT(*) FROM nodes WHERE project_id = ?1 AND vector IS NOT NULL LIMIT 1",
            rusqlite::params![engine.project_id()],
            |r| r.get::<_, i64>(0),
        )
        .map(|n| n > 0)
        .unwrap_or(false);
    let embedding_mode = if crate::neural_embed::is_neural_active() {
        "neural"
    } else if has_vectors {
        "semantic"
    } else {
        "fts"
    };
    Ok(serde_json::to_string_pretty(&json!({
        "indexing": {
            "in_progress": false,
            "total_nodes": total_nodes,
            "total_files": total_files
        },
        "capabilities": {
            "embedding_mode": embedding_mode
        }
    }))?)
}

fn tool_add_fact(engine: &HermesEngine, fact_type_str: &str, content: &str) -> Result<String> {
    let store = TemporalStore::new(engine.db().clone(), engine.project_id());
    let id = store.add_fact(None, FactType::parse_str(fact_type_str), content, None)?;
    Ok(serde_json::to_string_pretty(
        &json!({ "id": id, "status": "recorded" }),
    )?)
}

fn tool_list_facts(engine: &HermesEngine, filter: Option<&str>) -> Result<String> {
    let store = TemporalStore::new(engine.db().clone(), engine.project_id());
    let facts = store.get_active_facts(filter.map(FactType::parse_str).as_ref())?;
    Ok(serde_json::to_string_pretty(&facts)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HermesEngine;

    #[test]
    fn tool_mcp_status_returns_valid_json() {
        let engine = HermesEngine::in_memory("test-tool-status").unwrap();
        let result = tool_mcp_status(&engine).unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert!(parsed["indexing"]["total_nodes"].is_number());
        assert!(parsed["indexing"]["total_files"].is_number());
        assert!(parsed["capabilities"]["embedding_mode"].is_string());
    }

    #[test]
    fn tool_add_fact_and_list_facts() {
        let engine = HermesEngine::in_memory("test-tool-facts").unwrap();
        let result = tool_add_fact(&engine, "decision", "Use Rust").unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["status"], "recorded");
        assert!(parsed["id"].is_string());

        let list = tool_list_facts(&engine, None).unwrap();
        let facts: Value = serde_json::from_str(&list).unwrap();
        assert!(facts.as_array().unwrap().len() >= 1);
    }

    #[test]
    fn tool_list_facts_filtered() {
        let engine = HermesEngine::in_memory("test-tool-facts-filter").unwrap();
        tool_add_fact(&engine, "architecture", "Microservices design").unwrap();
        tool_add_fact(&engine, "decision", "Use Postgres").unwrap();

        let arch_facts = tool_list_facts(&engine, Some("architecture")).unwrap();
        let facts: Value = serde_json::from_str(&arch_facts).unwrap();
        for fact in facts.as_array().unwrap() {
            assert_ne!(fact["fact_type"].as_str().unwrap(), "");
        }
    }

    #[test]
    fn tool_stats_returns_valid_json() {
        let engine = HermesEngine::in_memory("test-tool-stats").unwrap();
        let result = tool_stats(&engine).unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert!(parsed["today"]["total_queries"].is_number());
        assert!(parsed["cumulative"]["total_queries"].is_number());
        assert!(parsed["today"]["savings_pct"].is_string());
    }

    #[test]
    fn tool_index_on_temp_dir() {
        let engine = HermesEngine::in_memory("test-tool-idx").unwrap();
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("lib.rs"), "pub fn hello() {}").unwrap();
        let result = tool_index(&engine, dir.path(), false).unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert!(parsed["total_files"].as_u64().unwrap() > 0);
    }

    #[test]
    fn tool_search_on_indexed_data() {
        let engine = HermesEngine::in_memory("test-tool-search").unwrap();
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("lib.rs"), "pub fn hello_world() {}").unwrap();
        tool_index(&engine, dir.path(), false).unwrap();
        let result = tool_search(&engine, "hello_world", dir.path(), dir.path().to_str().unwrap()).unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert!(!parsed["pointers"].as_array().unwrap().is_empty());
    }

    #[test]
    fn dispatch_initialize() {
        let engine = HermesEngine::in_memory("test-dispatch-init").unwrap();
        let root = std::path::PathBuf::from("/tmp/test");
        let cache = EngineCache::new(engine, root, vec![]);
        let result = dispatch(&cache, "initialize", &json!({})).unwrap();
        let info = &result["serverInfo"];
        assert_eq!(info["project_id"], "test-dispatch-init");
        assert_eq!(info["name"], "Hermes");
    }

    #[test]
    fn dispatch_tools_list() {
        let engine = HermesEngine::in_memory("test-dispatch-list").unwrap();
        let root = std::path::PathBuf::from("/tmp/test");
        let cache = EngineCache::new(engine, root, vec![]);
        let result = dispatch(&cache, "tools/list", &json!({})).unwrap();
        let tools = result["tools"].as_array().unwrap();
        assert!(!tools.is_empty());
    }

    #[test]
    fn dispatch_unknown_method_errors() {
        let engine = HermesEngine::in_memory("test-dispatch-err").unwrap();
        let root = std::path::PathBuf::from("/tmp/test");
        let cache = EngineCache::new(engine, root, vec![]);
        let result = dispatch(&cache, "nonexistent_method", &json!({}));
        assert!(result.is_err());
    }

    #[test]
    fn dispatch_unknown_tool_errors() {
        let engine = HermesEngine::in_memory("test-dispatch-tool-err").unwrap();
        let root = std::path::PathBuf::from("/tmp/test");
        let cache = EngineCache::new(engine, root, vec![]);
        let params = json!({"name": "nonexistent_tool", "arguments": {}});
        let result = dispatch(&cache, "tools/call", &params);
        assert!(result.is_err());
    }
}
