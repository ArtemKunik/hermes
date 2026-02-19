use anyhow::{bail, Result};
use hermes_engine::{
    accounting::{parse_since_duration, Accountant},
    graph::KnowledgeGraph,
    ingestion::IngestionPipeline,
    mcp_server,
    search::{SearchEngine, SearchMode},
    temporal::{FactType, TemporalStore},
    HermesEngine,
};
use std::{env, path::{Path, PathBuf}};

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        print_usage();
        return Ok(());
    }

    let (engine, project_root) = open_engine()?;
    let command = args[1].as_str();

    if command == "--stdio" {
        return mcp_server::run(&engine, &project_root);
    }

    match command {
        "index" => cmd_index(&engine, &project_root),
        "search" => {
            let query = args.get(2).map(String::as_str).unwrap_or("");
            if query.is_empty() {
                bail!("usage: hermes search <query>");
            }
            cmd_search(&engine, query)
        }
        "fetch" => {
            let id = args.get(2).map(String::as_str).unwrap_or("");
            if id.is_empty() {
                bail!("usage: hermes fetch <node_id>");
            }
            cmd_fetch(&engine, id)
        }
        "fact" => {
            let fact_type = args.get(2).map(String::as_str).unwrap_or("");
            let content = args.get(3).map(String::as_str).unwrap_or("");
            if fact_type.is_empty() || content.is_empty() {
                bail!("usage: hermes fact <type> <content>");
            }
            cmd_add_fact(&engine, fact_type, content)
        }
        "facts" => {
            let filter = args.get(2).map(String::as_str);
            cmd_list_facts(&engine, filter)
        }
        "stats" => {
            let since_arg = args.get(2).map(String::as_str);
            cmd_stats(&engine, since_arg)
        }
        unknown => {
            print_usage();
            bail!("unknown command: {unknown}");
        }
    }
}


fn open_engine() -> Result<(HermesEngine, PathBuf)> {
    let project_root = env::var("HERMES_PROJECT_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let db_path = env::var("HERMES_DB_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| project_root.join(".hermes.db"));

    let project_id = project_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    let engine = HermesEngine::new(&db_path, &project_id)?;
    Ok((engine, project_root))
}


fn cmd_index(engine: &HermesEngine, project_root: &Path) -> Result<()> {
    let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
    let pipeline = IngestionPipeline::new(&graph);
    let report = pipeline.ingest_directory(project_root)?;
    engine.invalidate_search_cache();
    let output = serde_json::json!({
        "total_files":  report.total_files,
        "indexed":      report.indexed,
        "skipped":      report.skipped,
        "errors":       report.errors,
        "nodes_created": report.nodes_created,
    });
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

fn cmd_search(engine: &HermesEngine, query: &str) -> Result<()> {
    let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
    let search = SearchEngine::new(&graph, engine.search_cache());
    let response = search.search(query, 10, &SearchMode::Smart)?;

    let acct = Accountant::new(engine.db().clone(), engine.project_id(), engine.session_id());
    acct.record_query(
        query,
        response.accounting.pointer_tokens,
        0,
        response.accounting.traditional_rag_estimate,
    )?;

    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}

fn cmd_fetch(engine: &HermesEngine, node_id: &str) -> Result<()> {
    let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
    let search = SearchEngine::new(&graph, engine.search_cache());

    let Some(response) = search.fetch(node_id)? else {
        bail!("node not found: {node_id}");
    };

    let traditional_estimate = response.token_count * 15;
    let acct = Accountant::new(engine.db().clone(), engine.project_id(), engine.session_id());
    acct.record_query(node_id, 0, response.token_count, traditional_estimate)?;

    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}

fn cmd_add_fact(engine: &HermesEngine, fact_type_str: &str, content: &str) -> Result<()> {
    let store = TemporalStore::new(engine.db().clone(), engine.project_id());
    let fact_type = FactType::parse_str(fact_type_str);
    let id = store.add_fact(None, fact_type, content, None)?;
    println!("{}", serde_json::json!({ "id": id, "status": "recorded" }));
    Ok(())
}

fn cmd_list_facts(engine: &HermesEngine, filter: Option<&str>) -> Result<()> {
    let store = TemporalStore::new(engine.db().clone(), engine.project_id());
    let fact_type = filter.map(FactType::parse_str);
    let facts = store.get_active_facts(fact_type.as_ref())?;
    println!("{}", serde_json::to_string_pretty(&facts)?);
    Ok(())
}

fn cmd_stats(engine: &HermesEngine, since_arg: Option<&str>) -> Result<()> {
    let acct       = Accountant::new(engine.db().clone(), engine.project_id(), engine.session_id());
    let session    = acct.get_session_stats()?;

    let since_dur = since_arg.and_then(parse_since_duration);
    let cumulative = acct.get_stats_since(since_dur)?;

    let since_label = since_arg.unwrap_or("all");
    let output = serde_json::json!({
        "project_id": engine.project_id(),
        "since_filter": since_label,
        "session": {
            "total_queries":            session.total_queries,
            "pointer_tokens_used":      session.total_pointer_tokens,
            "fetched_tokens_used":      session.total_fetched_tokens,
            "actual_tokens_total":      session.total_pointer_tokens + session.total_fetched_tokens,
            "traditional_rag_estimate": session.total_traditional_estimate,
            "tokens_saved":             session.cumulative_savings_tokens,
            "savings_pct":              format!("{:.1}%", session.cumulative_savings_pct),
        },
        "cumulative": {
            "total_queries":            cumulative.total_queries,
            "pointer_tokens_used":      cumulative.total_pointer_tokens,
            "fetched_tokens_used":      cumulative.total_fetched_tokens,
            "actual_tokens_total":      cumulative.total_pointer_tokens + cumulative.total_fetched_tokens,
            "traditional_rag_estimate": cumulative.total_traditional_estimate,
            "tokens_saved":             cumulative.cumulative_savings_tokens,
            "savings_pct":              format!("{:.1}%", cumulative.cumulative_savings_pct),
        },
    });
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}


fn print_usage() {
    eprintln!(
        "hermes — token-efficient code navigation\n\n\
         USAGE: hermes <command> [args]\n\n\
         Commands:\n\
           index               Re-index the project (run when files change)\n\
           search <query>      Search codebase; returns pointers (no full content)\n\
           fetch <node_id>     Fetch full content for a specific pointer\n\
           fact <type> <text>  Record a decision/learning (types: architecture, decision,\n\
                               learning, constraint, error_pattern, api_contract)\n\
           facts [type]        List active facts, optionally filtered by type\n\
           stats [--since <duration>]  Show token savings (--since: 24h, 7d, 30d, all)\n\
           --stdio             Run as MCP JSON-RPC 2.0 stdio server (for VS Code Copilot)\n\n\
         Env vars:\n\
           HERMES_PROJECT_ROOT             Root directory to index (default: cwd)\n\
           HERMES_DB_PATH                  SQLite DB path (default: <project_root>/.hermes.db)\n\
           HERMES_AUTO_INDEX_INTERVAL_SECS Re-index interval when running as MCP server\n\
                                           (default: 300 = 5 min; 0 = disabled)"
    );
}
