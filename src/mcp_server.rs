
use anyhow::Result;
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use crate::{
    accounting::Accountant,
    graph::KnowledgeGraph,
    ingestion::IngestionPipeline,
    search::{SearchEngine, SearchMode},
    temporal::{FactType, TemporalStore},
    HermesEngine,
};


fn spawn_auto_reindex(engine: HermesEngine, project_root: PathBuf) {
    let interval_secs = std::env::var("HERMES_AUTO_INDEX_INTERVAL_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(300);

    if interval_secs == 0 {
        eprintln!("[hermes] auto-reindex disabled (HERMES_AUTO_INDEX_INTERVAL_SECS=0)");
        return;
    }

    std::thread::spawn(move || {
        eprintln!("[hermes] auto-reindex thread started (interval={}s)", interval_secs);
        loop {
            std::thread::sleep(std::time::Duration::from_secs(interval_secs));
            let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
            let pipeline = IngestionPipeline::new(&graph);
            match pipeline.ingest_directory(&project_root) {
                Ok(report) => eprintln!(
                    "[hermes] auto-reindex complete: {} indexed, {} skipped, {} errors",
                    report.indexed, report.skipped, report.errors
                ),
                Err(e) => eprintln!("[hermes] auto-reindex failed: {}", e),
            }
        }
    });
}

pub fn run(engine: &HermesEngine, project_root: &Path) -> Result<()> {
    spawn_auto_reindex(engine.clone(), project_root.to_path_buf());

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                write_error(&mut out, &Value::Null, -32700, &format!("parse error: {e}"))?;
                continue;
            }
        };

        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        let method = msg["method"].as_str().unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(Value::Null);

        if method.starts_with("notifications/") {
            continue;
        }

        let result = dispatch(engine, project_root, method, &params);
        match result {
            Ok(payload) => write_ok(&mut out, &id, payload)?,
            Err(e) => write_error(&mut out, &id, -32603, &e.to_string())?,
        }
    }
    Ok(())
}


fn dispatch(
    engine: &HermesEngine,
    project_root: &Path,
    method: &str,
    params: &Value,
) -> Result<Value> {
    match method {
        "initialize" => Ok(handle_initialize()),
        "tools/list" => Ok(handle_tools_list()),
        "tools/call" => handle_tool_call(engine, project_root, params),
        other => anyhow::bail!("unknown method: {other}"),
    }
}


fn handle_initialize() -> Value {
    json!({
        "protocolVersion": "2024-11-05",
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": "Hermes", "version": env!("CARGO_PKG_VERSION") }
    })
}

fn handle_tools_list() -> Value {
    json!({
        "tools": [
            {
                "name": "hermes_search",
                "description": "Search the codebase knowledge graph. Returns pointers (not full content). Records token savings in accounting.",
                "inputSchema": {
                    "type": "object",
                    "properties": { "query": { "type": "string", "description": "Natural-language or keyword search query" } },
                    "required": ["query"]
                }
            },
            {
                "name": "hermes_fetch",
                "description": "Fetch full content for a specific knowledge-graph node by ID returned by hermes_search.",
                "inputSchema": {
                    "type": "object",
                    "properties": { "node_id": { "type": "string", "description": "Node ID from a previous search result" } },
                    "required": ["node_id"]
                }
            },
            {
                "name": "hermes_index",
                "description": "Re-index the project files into the knowledge graph. Run after adding or changing files.",
                "inputSchema": { "type": "object", "properties": {} }
            },
            {
                "name": "hermes_stats",
                "description": "Return cumulative token savings statistics across all Hermes sessions.",
                "inputSchema": { "type": "object", "properties": {} }
            },
            {
                "name": "hermes_fact",
                "description": "Record a persistent fact (decision, learning, constraint, etc.) into the temporal store.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "fact_type": { "type": "string", "description": "One of: architecture, decision, learning, constraint, error_pattern, api_contract" },
                        "content":   { "type": "string", "description": "The fact to record" }
                    },
                    "required": ["fact_type", "content"]
                }
            },
            {
                "name": "hermes_facts",
                "description": "List active facts from the temporal store, optionally filtered by type.",
                "inputSchema": {
                    "type": "object",
                    "properties": { "fact_type": { "type": "string", "description": "Optional filter type (omit for all)" } }
                }
            }
        ]
    })
}

fn handle_tool_call(engine: &HermesEngine, project_root: &Path, params: &Value) -> Result<Value> {
    let name = params["name"].as_str().unwrap_or("");
    let args = &params["arguments"];

    let text = match name {
        "hermes_search" => {
            let query = args["query"].as_str().unwrap_or("");
            anyhow::ensure!(!query.is_empty(), "hermes_search requires 'query'");
            tool_search(engine, query)?
        }
        "hermes_fetch" => {
            let node_id = args["node_id"].as_str().unwrap_or("");
            anyhow::ensure!(!node_id.is_empty(), "hermes_fetch requires 'node_id'");
            tool_fetch(engine, node_id)?
        }
        "hermes_index"  => tool_index(engine, project_root)?,
        "hermes_stats"  => tool_stats(engine)?,
        "hermes_fact"   => {
            let ft = args["fact_type"].as_str().unwrap_or("");
            let c  = args["content"].as_str().unwrap_or("");
            anyhow::ensure!(!ft.is_empty() && !c.is_empty(), "hermes_fact requires 'fact_type' and 'content'");
            tool_add_fact(engine, ft, c)?
        }
        "hermes_facts" => {
            let filter = args["fact_type"].as_str();
            tool_list_facts(engine, filter)?
        }
        other => anyhow::bail!("unknown tool: {other}"),
    };

    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
}


fn tool_search(engine: &HermesEngine, query: &str) -> Result<String> {
    let graph  = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
    let search = SearchEngine::new(&graph, engine.search_cache());
    let resp   = search.search(query, 10, &SearchMode::Smart)?;
    let acct   = Accountant::new(engine.db().clone(), engine.project_id(), engine.session_id());
    acct.record_query(query, resp.accounting.pointer_tokens, 0, resp.accounting.traditional_rag_estimate)?;
    Ok(serde_json::to_string_pretty(&resp)?)
}

fn tool_fetch(engine: &HermesEngine, node_id: &str) -> Result<String> {
    let graph  = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
    let search = SearchEngine::new(&graph, engine.search_cache());
    let Some(resp) = search.fetch(node_id)? else {
        anyhow::bail!("node not found: {node_id}");
    };
    let acct = Accountant::new(engine.db().clone(), engine.project_id(), engine.session_id());
    acct.record_query(node_id, 0, resp.token_count, resp.token_count * 15)?;
    Ok(serde_json::to_string_pretty(&resp)?)
}

fn tool_index(engine: &HermesEngine, project_root: &Path) -> Result<String> {
    let graph    = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
    let pipeline = IngestionPipeline::new(&graph);
    let report   = pipeline.ingest_directory(project_root)?;
    engine.invalidate_search_cache();
    Ok(serde_json::to_string_pretty(&json!({
        "total_files": report.total_files, "indexed": report.indexed,
        "skipped": report.skipped, "errors": report.errors,
        "nodes_created": report.nodes_created,
    }))?)
}

fn tool_stats(engine: &HermesEngine) -> Result<String> {
    let acct = Accountant::new(engine.db().clone(), engine.project_id(), engine.session_id());
    let today      = acct.get_today_stats()?;
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

fn tool_add_fact(engine: &HermesEngine, fact_type_str: &str, content: &str) -> Result<String> {
    let store = TemporalStore::new(engine.db().clone(), engine.project_id());
    let id = store.add_fact(None, FactType::parse_str(fact_type_str), content, None)?;
    Ok(serde_json::to_string_pretty(&json!({ "id": id, "status": "recorded" }))?)
}

fn tool_list_facts(engine: &HermesEngine, filter: Option<&str>) -> Result<String> {
    let store = TemporalStore::new(engine.db().clone(), engine.project_id());
    let facts = store.get_active_facts(filter.map(FactType::parse_str).as_ref())?;
    Ok(serde_json::to_string_pretty(&facts)?)
}


fn write_ok(out: &mut impl Write, id: &Value, result: Value) -> Result<()> {
    let envelope = json!({ "jsonrpc": "2.0", "id": id, "result": result });
    writeln!(out, "{}", serde_json::to_string(&envelope)?)?;
    out.flush()?;
    Ok(())
}

fn write_error(out: &mut impl Write, id: &Value, code: i32, message: &str) -> Result<()> {
    let envelope = json!({
        "jsonrpc": "2.0", "id": id,
        "error": { "code": code, "message": message }
    });
    writeln!(out, "{}", serde_json::to_string(&envelope)?)?;
    out.flush()?;
    Ok(())
}
