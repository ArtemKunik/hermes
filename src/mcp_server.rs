use anyhow::Result;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::{
    accounting::Accountant,
    graph::KnowledgeGraph,
    ingestion::IngestionPipeline,
    mcp_tools_validation::{tool_check_consistency, tool_validate_env},
    search::{SearchEngine, SearchMode},
    temporal::{FactType, TemporalStore},
    HermesEngine,
};

// Registry entry: a pre-registered project path (from HERMES_PROJECTS env var).
// Stored as the canonical absolute path; project_id is its basename.
struct RegistryEntry {
    canonical: PathBuf,
    project_id: String,
}

// Caches engines keyed by canonicalized project root.
// A single MCP process can serve any number of repositories via:
//   1. HERMES_PROJECTS env var — pre-register paths at startup
//   2. Passing a project name (basename) or full path as project_root per call
struct EngineCache {
    default_engine: HermesEngine,
    default_root: PathBuf,
    extra: Mutex<HashMap<PathBuf, HermesEngine>>,
    // name → (canonical path, project_id) for HERMES_PROJECTS-registered projects
    registry: Vec<RegistryEntry>,
}

impl EngineCache {
    fn new(engine: HermesEngine, root: PathBuf, registry: Vec<RegistryEntry>) -> Self {
        Self {
            default_engine: engine,
            default_root: root,
            extra: Mutex::new(HashMap::new()),
            registry,
        }
    }

    // Resolve a project_root argument to (engine, canonical_path).
    // Accepts:
    //   - a project name / basename  ("lonaspark")
    //   - an absolute path           ("D:/source/lonaspark")
    // Registry entries (from HERMES_PROJECTS) are checked by name first.
    fn resolve(&self, project_root_arg: Option<&str>) -> Result<(HermesEngine, PathBuf)> {
        let Some(arg) = project_root_arg.filter(|s| !s.is_empty()) else {
            return Ok((self.default_engine.clone(), self.default_root.clone()));
        };

        // 1. Registry lookup by name (basename) — lets agents pass "lonaspark" not a full path
        let effective_root: PathBuf = if let Some(entry) = self.registry_lookup(arg) {
            eprintln!(
                "[hermes] resolve: '{}' matched registry entry project_id={}",
                arg, entry.project_id
            );
            entry.canonical.clone()
        } else {
            PathBuf::from(arg)
        };

        // 2. Canonicalize the path (best-effort; keep original if path doesn't exist yet)
        let canonical = effective_root
            .canonicalize()
            .unwrap_or_else(|_| effective_root.clone());

        // 3. Default engine check
        let default_canonical = self
            .default_root
            .canonicalize()
            .unwrap_or_else(|_| self.default_root.clone());

        if canonical == default_canonical {
            eprintln!(
                "[hermes] resolve: '{}' -> default project_id={}",
                arg,
                self.default_engine.project_id()
            );
            return Ok((self.default_engine.clone(), self.default_root.clone()));
        }

        // 4. Extra cache lookup
        let mut cache = self.extra.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        if let Some(engine) = cache.get(&canonical) {
            eprintln!(
                "[hermes] resolve: '{}' -> cached project_id={}",
                arg,
                engine.project_id()
            );
            return Ok((engine.clone(), canonical));
        }

        // 5. Open a new engine, auto-index the directory
        let project_id = canonical
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();
        let engine = HermesEngine::new(&canonical.join(".hermes.db"), &project_id)?;
        eprintln!(
            "[hermes] resolve: '{}' -> new project_id={} at {}, auto-indexing...",
            arg,
            project_id,
            canonical.display()
        );
        let graph = KnowledgeGraph::new(engine.db().clone(), &project_id);
        match IngestionPipeline::new(&graph).ingest_directory(&canonical) {
            Ok(r) => eprintln!(
                "[hermes] auto-index {}: {} indexed, {} skipped, {} errors",
                project_id, r.indexed, r.skipped, r.errors
            ),
            Err(e) => eprintln!("[hermes] auto-index {} failed: {e}", project_id),
        }
        cache.insert(canonical.clone(), engine.clone());
        Ok((engine, canonical))
    }

    fn registry_lookup(&self, arg: &str) -> Option<&RegistryEntry> {
        // Exact project_id (basename) match
        if let Some(entry) = self.registry.iter().find(|e| e.project_id == arg) {
            return Some(entry);
        }
        // Canonical path match (arg is an absolute path already in the registry)
        let candidate = PathBuf::from(arg)
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(arg));
        self.registry.iter().find(|e| e.canonical == candidate)
    }

    fn list_projects(&self) -> Vec<Value> {
        let mut projects = vec![json!({
            "project_id": self.default_engine.project_id(),
            "project_root": self.default_root.display().to_string(),
            "source": "default"
        })];

        for entry in &self.registry {
            if entry.canonical
                != self
                    .default_root
                    .canonicalize()
                    .unwrap_or_else(|_| self.default_root.clone())
            {
                projects.push(json!({
                    "project_id": entry.project_id,
                    "project_root": entry.canonical.display().to_string(),
                    "source": "HERMES_PROJECTS"
                }));
            }
        }

        if let Ok(extra) = self.extra.lock() {
            let default_canonical = self
                .default_root
                .canonicalize()
                .unwrap_or_else(|_| self.default_root.clone());
            for (path, engine) in extra.iter() {
                let is_registry = self.registry.iter().any(|e| &e.canonical == path);
                if path != &default_canonical && !is_registry {
                    projects.push(json!({
                        "project_id": engine.project_id(),
                        "project_root": path.display().to_string(),
                        "source": "auto-discovered"
                    }));
                }
            }
        }

        projects
    }
}

// Parse HERMES_PROJECTS env var: semicolon-separated absolute paths.
// Example: HERMES_PROJECTS=D:\source\lonaspark;D:\source\hermes
fn parse_project_registry() -> Vec<RegistryEntry> {
    let Ok(raw) = std::env::var("HERMES_PROJECTS") else {
        return Vec::new();
    };
    raw.split(';')
        .filter(|s| !s.trim().is_empty())
        .filter_map(|s| {
            let path = PathBuf::from(s.trim());
            let canonical = path.canonicalize().unwrap_or_else(|_| path);
            let project_id = canonical
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
            Some(RegistryEntry { canonical, project_id })
        })
        .collect()
}

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
        eprintln!(
            "[hermes] auto-reindex thread started (interval={}s)",
            interval_secs
        );
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
        eprintln!("[hermes] registered projects: {}", names.join(", "));
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

        let result = dispatch(&cache, method, &params);
        match result {
            Ok(payload) => write_ok(&mut out, &id, payload)?,
            Err(e) => write_error(&mut out, &id, -32603, &e.to_string())?,
        }
    }
    Ok(())
}

fn dispatch(cache: &EngineCache, method: &str, params: &Value) -> Result<Value> {
    match method {
        "initialize" => Ok(handle_initialize(cache)),
        "tools/list" => Ok(handle_tools_list()),
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

fn project_root_schema() -> Value {
    json!({
        "type": "string",
        "description": "Project name (e.g. 'lonaspark') or absolute path. Use hermes_list_projects to see available projects. Wrong value → wrong repo's results."
    })
}

fn handle_tools_list() -> Value {
    json!({
        "tools": [
            {
                "name": "hermes_list_projects",
                "description": "List all projects known to this hermes server (default + HERMES_PROJECTS registry + previously accessed). Use the returned project_root values for all other hermes tools.",
                "inputSchema": {
                    "type": "object",
                    "properties": {}
                }
            },
            {
                "name": "hermes_search",
                "description": "Search the codebase knowledge graph. Returns pointers (not full content). Records token savings in accounting.",
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
                "name": "hermes_fetch",
                "description": "Fetch full content for a specific knowledge-graph node by ID returned by hermes_search.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "node_id": { "type": "string", "description": "Node ID from a previous search result" },
                        "project_root": project_root_schema()
                    },
                    "required": ["node_id", "project_root"]
                }
            },
            {
                "name": "hermes_index",
                "description": "Re-index the project files into the knowledge graph. Run after adding or changing files.",
                "inputSchema": {
                    "type": "object",
                    "properties": { "project_root": project_root_schema() },
                    "required": ["project_root"]
                }
            },
            {
                "name": "hermes_stats",
                "description": "Return cumulative token savings statistics across all Hermes sessions.",
                "inputSchema": {
                    "type": "object",
                    "properties": { "project_root": project_root_schema() },
                    "required": ["project_root"]
                }
            },
            {
                "name": "hermes_fact",
                "description": "Record a persistent fact (decision, learning, constraint, etc.) into the temporal store.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "fact_type": { "type": "string", "description": "One of: architecture, decision, learning, constraint, error_pattern, api_contract" },
                        "content":   { "type": "string", "description": "The fact to record" },
                        "project_root": project_root_schema()
                    },
                    "required": ["fact_type", "content", "project_root"]
                }
            },
            {
                "name": "hermes_facts",
                "description": "List active facts from the temporal store, optionally filtered by type.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "fact_type": { "type": "string", "description": "Optional filter type (omit for all)" },
                        "project_root": project_root_schema()
                    },
                    "required": ["project_root"]
                }
            },
            {
                "name": "hermes_validate_env",
                "description": "Validate an environment variable name against the config_registry populated during hermes_index. Returns valid:true when the name is known, or valid:false with up to 5 Levenshtein-closest suggestions.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "env_var": { "type": "string", "description": "The environment variable name to validate (e.g. DATABASE_URL)" },
                        "project_root": project_root_schema()
                    },
                    "required": ["env_var", "project_root"]
                }
            },
            {
                "name": "hermes_check_consistency",
                "description": "Scan config_registry for env vars that are used in code but not defined (unknown) or defined but never referenced (unused). Run after hermes_index.",
                "inputSchema": {
                    "type": "object",
                    "properties": { "project_root": project_root_schema() },
                    "required": ["project_root"]
                }
            }
        ]
    })
}

fn handle_tool_call(cache: &EngineCache, params: &Value) -> Result<Value> {
    let name = params["name"].as_str().unwrap_or("");
    let args = &params["arguments"];

    // hermes_list_projects needs no project_root
    if name == "hermes_list_projects" {
        let text = tool_list_projects(cache)?;
        return Ok(json!({ "content": [{ "type": "text", "text": text }] }));
    }

    let project_root_arg = args["project_root"].as_str().unwrap_or("");
    anyhow::ensure!(
        !project_root_arg.is_empty(),
        "project_root is required — pass a project name (e.g. 'lonaspark') or absolute path. \
         Call hermes_list_projects to see available projects."
    );
    let (engine, project_root) = cache.resolve(Some(project_root_arg))?;

    eprintln!(
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
        "hermes_index" => tool_index(&engine, &project_root)?,
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
            eprintln!("[hermes] WARNING: {}", warning);
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
    acct.record_query(node_id, 0, resp.token_count, resp.token_count * 15)?;
    let mut out = serde_json::to_value(&resp)?;
    out["project_id"] = json!(engine.project_id());
    out["project_root"] = json!(project_root.display().to_string());
    Ok(serde_json::to_string_pretty(&out)?)
}

fn tool_index(engine: &HermesEngine, project_root: &Path) -> Result<String> {
    let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
    let pipeline = IngestionPipeline::new(&graph);
    let report = pipeline.ingest_directory(project_root)?;
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
