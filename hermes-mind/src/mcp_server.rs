use anyhow::Result;
use hermes_engine::{
    accounting::Accountant,
    graph::KnowledgeGraph,
    search::{SearchEngine, SearchMode},
    HermesEngine,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::connectors::Connector;
use crate::sync_state::SyncState;

struct EngineCache {
    default_engine: HermesEngine,
    default_root: PathBuf,
    extra: Mutex<HashMap<PathBuf, HermesEngine>>,
}

impl EngineCache {
    fn new(engine: HermesEngine, root: PathBuf) -> Self {
        Self {
            default_engine: engine,
            default_root: root,
            extra: Mutex::new(HashMap::new()),
        }
    }

    fn resolve(&self, project_root_arg: Option<&str>) -> Result<(HermesEngine, PathBuf)> {
        let Some(root_str) = project_root_arg.filter(|s| !s.is_empty()) else {
            return Ok((self.default_engine.clone(), self.default_root.clone()));
        };

        let root = PathBuf::from(root_str);
        let root = root.canonicalize().unwrap_or_else(|_| root.clone());
        let default_canonical = self
            .default_root
            .canonicalize()
            .unwrap_or_else(|_| self.default_root.clone());

        if root == default_canonical {
            return Ok((self.default_engine.clone(), self.default_root.clone()));
        }

        let mut cache = self.extra.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        if let Some(engine) = cache.get(&root) {
            return Ok((engine.clone(), root.clone()));
        }

        let project_id = root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();
        let engine = HermesEngine::new(&root.join(".hermes-mind.db"), &project_id)?;
        cache.insert(root.clone(), engine.clone());
        Ok((engine, root))
    }
}

pub fn run(
    engine: &HermesEngine,
    project_root: &Path,
    connectors: Vec<Box<dyn Connector>>,
) -> Result<()> {
    let cache = EngineCache::new(engine.clone(), project_root.to_path_buf());
    let connectors: std::sync::Arc<Vec<Box<dyn Connector>>> = std::sync::Arc::new(connectors);

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

        let result = dispatch(&cache, &connectors, method, &params);
        match result {
            Ok(payload) => write_ok(&mut out, &id, payload)?,
            Err(e) => write_error(&mut out, &id, -32603, &e.to_string())?,
        }
    }
    Ok(())
}

fn dispatch(
    cache: &EngineCache,
    connectors: &[Box<dyn Connector>],
    method: &str,
    params: &Value,
) -> Result<Value> {
    match method {
        "initialize" => Ok(handle_initialize()),
        "tools/list" => Ok(handle_tools_list()),
        "tools/call" => handle_tool_call(cache, connectors, params),
        other => anyhow::bail!("unknown method: {other}"),
    }
}

fn handle_initialize() -> Value {
    json!({
        "protocolVersion": "2024-11-05",
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": "HermesMind", "version": env!("CARGO_PKG_VERSION") }
    })
}

fn project_root_schema() -> Value {
    json!({
        "type": "string",
        "description": "Absolute path to the project root (determines which .hermes-mind.db to use)."
    })
}

fn handle_tools_list() -> Value {
    json!({
        "tools": [
            {
                "name": "mind_search",
                "description": "Search personal knowledge (messages, emails, documents) across all synced connectors.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Natural-language or keyword search query" },
                        "project_root": project_root_schema()
                    },
                    "required": ["query", "project_root"]
                }
            },
            {
                "name": "mind_fetch",
                "description": "Fetch full content of a personal knowledge node by ID.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "node_id": { "type": "string", "description": "Node ID from a previous mind_search result" },
                        "project_root": project_root_schema()
                    },
                    "required": ["node_id", "project_root"]
                }
            },
            {
                "name": "mind_sync",
                "description": "Trigger a sync for all connectors (or a specific one). Ingests new messages, emails, and documents.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "project_root": project_root_schema(),
                        "connector": {
                            "type": "string",
                            "description": "Optional connector name to sync (whatsapp, gmail, onedrive, telegram). Omit for all."
                        }
                    },
                    "required": ["project_root"]
                }
            },
            {
                "name": "mind_status",
                "description": "List active connectors and their configuration status.",
                "inputSchema": {
                    "type": "object",
                    "properties": { "project_root": project_root_schema() },
                    "required": ["project_root"]
                }
            }
        ]
    })
}

fn handle_tool_call(
    cache: &EngineCache,
    connectors: &[Box<dyn Connector>],
    params: &Value,
) -> Result<Value> {
    let name = params["name"].as_str().unwrap_or("");
    let args = &params["arguments"];

    let project_root_arg = args["project_root"].as_str().unwrap_or("");
    anyhow::ensure!(
        !project_root_arg.is_empty(),
        "project_root is required — pass the absolute path to the repository root"
    );
    let (engine, project_root) = cache.resolve(Some(project_root_arg))?;

    let text = match name {
        "mind_search" => {
            let query = args["query"].as_str().unwrap_or("");
            anyhow::ensure!(!query.is_empty(), "mind_search requires 'query'");
            tool_search(&engine, query, &project_root)?
        }
        "mind_fetch" => {
            let node_id = args["node_id"].as_str().unwrap_or("");
            anyhow::ensure!(!node_id.is_empty(), "mind_fetch requires 'node_id'");
            tool_fetch(&engine, node_id, &project_root)?
        }
        "mind_sync" => {
            let filter = args["connector"].as_str();
            tool_sync(&engine, &project_root, connectors, filter)?
        }
        "mind_status" => tool_status(connectors)?,
        other => anyhow::bail!("unknown tool: {other}"),
    };

    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
}

fn tool_search(engine: &HermesEngine, query: &str, project_root: &Path) -> Result<String> {
    let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
    let search = SearchEngine::new(&graph, engine.search_cache());
    let resp = search.search(query, 10, &SearchMode::Smart)?;
    let acct = Accountant::new(engine.db().clone(), engine.project_id(), engine.session_id());
    acct.record_query(query, resp.accounting.pointer_tokens, 0, resp.accounting.traditional_rag_estimate)?;
    let mut out = serde_json::to_value(&resp)?;
    out["project_id"] = json!(engine.project_id());
    out["project_root"] = json!(project_root.display().to_string());
    Ok(serde_json::to_string_pretty(&out)?)
}

fn tool_fetch(engine: &HermesEngine, node_id: &str, project_root: &Path) -> Result<String> {
    let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
    let search = SearchEngine::new(&graph, engine.search_cache());
    let Some(resp) = search.fetch(node_id)? else {
        anyhow::bail!("node not found: {node_id}");
    };
    let acct = Accountant::new(engine.db().clone(), engine.project_id(), engine.session_id());
    acct.record_query(node_id, 0, resp.token_count, resp.token_count * 15)?;
    let mut out = serde_json::to_value(&resp)?;
    out["project_id"] = json!(engine.project_id());
    out["project_root"] = json!(project_root.display().to_string());
    Ok(serde_json::to_string_pretty(&out)?)
}

fn tool_sync(
    engine: &HermesEngine,
    project_root: &Path,
    connectors: &[Box<dyn Connector>],
    filter: Option<&str>,
) -> Result<String> {
    if connectors.is_empty() {
        return Ok(serde_json::to_string_pretty(&json!({
            "status": "no connectors configured",
            "hint": "set HERMES_MIND_GMAIL_REFRESH_TOKEN, HERMES_MIND_WA_SIDECAR_PATH, etc."
        }))?);
    }

    let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
    let sync_state = SyncState::new(engine.db().clone(), engine.project_id());
    let mut results = Vec::new();

    for connector in connectors {
        if let Some(name) = filter {
            if connector.name() != name {
                continue;
            }
        }
        let outcome = match connector.sync(&graph, &sync_state) {
            Ok(r) => json!({
                "connector": connector.name(),
                "ingested": r.ingested,
                "skipped": r.skipped,
                "errors": r.errors,
            }),
            Err(e) => json!({
                "connector": connector.name(),
                "error": e.to_string(),
            }),
        };
        results.push(outcome);
    }

    engine.invalidate_search_cache();
    Ok(serde_json::to_string_pretty(&json!({
        "project_id": engine.project_id(),
        "project_root": project_root.display().to_string(),
        "results": results,
    }))?)
}

fn tool_status(connectors: &[Box<dyn Connector>]) -> Result<String> {
    let names: Vec<&str> = connectors.iter().map(|c| c.name()).collect();
    Ok(serde_json::to_string_pretty(&json!({
        "active_connectors": names,
        "available": ["whatsapp", "gmail", "onedrive", "telegram"],
    }))?)
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
