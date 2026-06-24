use anyhow::Result;
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::path::PathBuf;
use tracing::{error, info};

use crate::{
    accounting::Accountant,
    engine_cache::{EngineCache, parse_project_registry},
    graph::KnowledgeGraph,
    ingestion::IngestionPipeline,
    mcp_tools_validation::{tool_check_consistency, tool_validate_env},
    search::{SearchEngine, SearchMode},
    temporal::TemporalStore,
    temporal_types::{AddFactInput, FactType},
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
    let registry = parse_project_registry();
    if !registry.is_empty() {
        let names: Vec<&str> = registry.iter().map(|e| e.project_id.as_str()).collect();
        info!("[hermes] registered projects: {}", names.join(", "));
    }
    let cache = EngineCache::new(engine.clone(), project_root.to_path_buf(), registry);

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

        let result = crate::mcp_tools::dispatch(&cache, method, &params);
        match result {
            Ok(payload) => write_ok(&mut out, &id, payload)?,
            Err(e) => write_error(&mut out, &id, -32603, &e.to_string())?,
        }
    }
    Ok(())
}

/// Serve the MCP JSON-RPC API over plain HTTP on the given port.
/// Accepts POST /api/mcp with a JSON-RPC 2.0 body and returns a JSON-RPC 2.0 response.
pub fn run_http(engine: &HermesEngine, project_root: &Path, port: u16) -> Result<()> {
    use std::sync::Arc;
    spawn_auto_reindex(engine.clone(), project_root.to_path_buf());
    let registry = parse_project_registry();
    if !registry.is_empty() {
        let names: Vec<&str> = registry.iter().map(|e| e.project_id.as_str()).collect();
        info!("[hermes] registered projects: {}", names.join(", "));
    }
    let cache = Arc::new(EngineCache::new(
        engine.clone(),
        project_root.to_path_buf(),
        registry,
    ));

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async move {
        let addr = format!("[::]:{port}");
        let listener = match tokio::net::TcpListener::bind(&addr).await {
            Ok(l) => {
                info!("[hermes] HTTP MCP listening on http://localhost:{port}/api/mcp (dual-stack)");
                l
            }
            Err(_) => {
                let addr4 = format!("0.0.0.0:{port}");
                let l = tokio::net::TcpListener::bind(&addr4).await?;
                info!("[hermes] HTTP MCP listening on http://localhost:{port}/api/mcp (IPv4 only)");
                l
            }
        };
        loop {
            let (stream, _peer) = listener.accept().await?;
            let cache = Arc::clone(&cache);
            tokio::spawn(async move {
                if let Err(e) = handle_http_conn(stream, cache).await {
                    error!("[hermes] http conn error: {e}");
                }
            });
        }
        #[allow(unreachable_code)]
        Ok::<(), anyhow::Error>(())
    })?;
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
            },
            {
                "name": "hermes_validate_env",
                "description": "Validate an environment variable name against the config_registry populated during hermes_index.",
                "inputSchema": {
                    "type": "object",
                    "properties": { "env_var": { "type": "string", "description": "The environment variable name to validate" } },
                    "required": ["env_var"]
                }
            },
            {
                "name": "hermes_check_consistency",
                "description": "Scan config_registry for env vars that are used in code but not defined (unknown) or defined but never referenced (unused).",
                "inputSchema": { "type": "object", "properties": {} }
            },
            {
                "name": "hermes_impact",
                "description": "Return a qualitative impact summary of Hermes token savings.",
                "inputSchema": { "type": "object", "properties": {} }
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
        "hermes_validate_env" => {
            let var = args["env_var"].as_str().unwrap_or("");
            anyhow::ensure!(!var.is_empty(), "hermes_validate_env requires 'env_var'");
            tool_validate_env(engine, var)?
        }
        "hermes_check_consistency" => tool_check_consistency(engine)?,
        "hermes_impact" => tool_impact(engine)?,
        other => anyhow::bail!("unknown tool: {other}"),
    };

    Ok(json!(text))
}

async fn handle_http_conn(
    stream: tokio::net::TcpStream,
    cache: std::sync::Arc<EngineCache>,
) -> Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = stream;

    let mut raw = Vec::with_capacity(4096);
    let mut tmp = [0u8; 4096];
    let header_end = loop {
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            return Ok(());
        }
        raw.extend_from_slice(&tmp[..n]);
        if let Some(pos) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos;
        }
        if raw.len() > 1_048_576 {
            anyhow::bail!("headers too large");
        }
    };

    let headers_text = std::str::from_utf8(&raw[..header_end]).unwrap_or("");
    let first_line = headers_text.lines().next().unwrap_or("");

    if first_line.starts_with("OPTIONS") {
        let resp = b"HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\n\r\n";
        stream.write_all(resp).await?;
        return Ok(());
    }

    let content_length: usize = headers_text
        .lines()
        .find(|l| l.to_lowercase().starts_with("content-length:"))
        .and_then(|l| l.splitn(2, ':').nth(1))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0);

    let body_start = header_end + 4;
    let mut body = raw[body_start..].to_vec();
    while body.len() < content_length {
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&tmp[..n]);
    }
    body.truncate(content_length);

    let msg: Value = serde_json::from_slice(&body)
        .map_err(|e| anyhow::anyhow!("JSON parse error: {e}"))?;
    let id = msg.get("id").cloned().unwrap_or(Value::Null);
    let method = msg["method"].as_str().unwrap_or("");
    let params = msg.get("params").cloned().unwrap_or(Value::Null);

    if method.starts_with("notifications/") {
        stream
            .write_all(b"HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\n\r\n")
            .await?;
        return Ok(());
    }

    let cache_clone = cache.clone();
    let method_owned = method.to_string();
    let params_owned = params.clone();
    let dispatch_result = tokio::task::spawn_blocking(move || {
        crate::mcp_tools::dispatch(&cache_clone, &method_owned, &params_owned)
    })
    .await
    .map_err(|e| anyhow::anyhow!("spawn_blocking panic: {e}"))?;

    let response_body = match dispatch_result {
        Ok(payload) => serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": payload,
        }))?,
        Err(e) => serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": -32603, "message": e.to_string()},
        }))?,
    };

    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\n\r\n",
        response_body.len()
    );
    stream.write_all(header.as_bytes()).await?;
    stream.write_all(&response_body).await?;
    Ok(())
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
    let cumulative = acct.get_cumulative_stats()?;
    let today = acct.get_stats_since(Some(std::time::Duration::from_secs(86400)))?;
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
    let input = AddFactInput {
        node_id: None,
        fact_type: FactType::parse_str(fact_type_str),
        content,
        topic: None,
        tags: vec![],
        confidence: None,
        ttl: None,
        source_reference: None,
        provenance: None,
        repo_id: None,
        agent_id: None,
    };
    let id = store.add_fact(input)?;
    Ok(serde_json::to_string_pretty(&json!({ "id": id, "status": "recorded" }))?)
}

fn tool_list_facts(engine: &HermesEngine, filter_str: Option<&str>) -> Result<String> {
    use crate::temporal_types::FactFilter;
    let store = TemporalStore::new(engine.db().clone(), engine.project_id());
    let filter = FactFilter {
        fact_type: filter_str.map(FactType::parse_str),
        ..Default::default()
    };
    let facts = store.get_active_facts(&filter)?;
    Ok(serde_json::to_string_pretty(&facts)?)
}

fn tool_impact(engine: &HermesEngine) -> Result<String> {
    let acct = Accountant::new(engine.db().clone(), engine.project_id(), engine.session_id());
    let summary = acct.get_impact_summary()?;
    Ok(serde_json::to_string_pretty(&summary)?)
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn write_ok_produces_jsonrpc_envelope() {
        let mut buf = Vec::new();
        write_ok(&mut buf, &json!(42), json!({"data": "hello"})).unwrap();
        let output = String::from_utf8(buf).unwrap();
        let parsed: Value = serde_json::from_str(output.trim()).unwrap();
        assert_eq!(parsed["jsonrpc"], "2.0");
        assert_eq!(parsed["id"], 42);
        assert_eq!(parsed["result"]["data"], "hello");
    }

    #[test]
    fn write_ok_with_null_id() {
        let mut buf = Vec::new();
        write_ok(&mut buf, &Value::Null, json!({"status": "ok"})).unwrap();
        let output = String::from_utf8(buf).unwrap();
        let parsed: Value = serde_json::from_str(output.trim()).unwrap();
        assert_eq!(parsed["jsonrpc"], "2.0");
        assert!(parsed["id"].is_null());
        assert_eq!(parsed["result"]["status"], "ok");
    }

    #[test]
    fn write_error_produces_error_envelope() {
        let mut buf = Vec::new();
        write_error(&mut buf, &json!(1), -32603, "internal error").unwrap();
        let output = String::from_utf8(buf).unwrap();
        let parsed: Value = serde_json::from_str(output.trim()).unwrap();
        assert_eq!(parsed["jsonrpc"], "2.0");
        assert_eq!(parsed["id"], 1);
        assert_eq!(parsed["error"]["code"], -32603);
        assert_eq!(parsed["error"]["message"], "internal error");
    }
}
