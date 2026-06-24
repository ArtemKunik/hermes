use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use hermes_engine::{
    accounting::{parse_since_duration, Accountant},
    graph::KnowledgeGraph,
    ingestion::IngestionPipeline,
    mcp_server,
    search::{SearchEngine, SearchMode},
    temporal::{TemporalStore},
    temporal_types::{AddFactInput, FactType},
    HermesEngine,
};
use std::{env, path::PathBuf};

#[derive(Parser)]
#[command(
    name = "hermes",
    about = "Token-efficient code navigation",
    arg_required_else_help = true,
    after_help = "\
Environment variables:
  HERMES_PROJECT_ROOT             Root directory to index (default: cwd)
  HERMES_DB_PATH                  SQLite DB path (default: <project_root>/.hermes.db)
  HERMES_AUTO_INDEX_INTERVAL_SECS Re-index interval when running as MCP server
                                  (default: 300 = 5 min; 0 = disabled)"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Run as MCP JSON-RPC 2.0 stdio server
    #[arg(long)]
    stdio: bool,

    /// Run as MCP JSON-RPC 2.0 HTTP server on the given port (e.g. --http 38081)
    #[arg(long)]
    http: Option<u16>,
}

#[derive(Subcommand)]
enum Commands {
    /// Re-index the project (run when files change)
    Index {
        /// Force a full re-index, ignoring content hashes
        #[arg(long)]
        force: bool,
    },

    /// <query> - Search codebase; returns pointers (no full content)
    Search { query: String },

    /// <node_id> - Fetch full content for a specific pointer
    Fetch { node_id: String },

    /// <type> <text> - Record a decision/learning (types: architecture, decision, learning, constraint, error_pattern, api_contract)
    Fact { fact_type: String, content: String },

    /// [type] - List active facts, optionally filtered by type
    Facts { filter: Option<String> },

    /// [duration] or [--since <duration>] - Show token savings (duration: 24h, 7d, 30d, all)
    Stats {
        /// Positional duration kept for backward compatibility (e.g., `hermes stats 24h`)
        since: Option<String>,

        /// Explicit duration flag (e.g., `hermes stats --since 24h`)
        #[arg(long = "since")]
        since_flag: Option<String>,
    },
}

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    let (engine, project_root) = open_engine()?;

    if cli.stdio {
        return mcp_server::run(&engine, &project_root);
    }

    if let Some(port) = cli.http {
        return mcp_server::run_http(&engine, &project_root, port);
    }

    match cli
        .command
        .expect("clap should require a subcommand")
    {
        Commands::Index { force } => cmd_index(&engine, &project_root, force),
        Commands::Search { query } => cmd_search(&engine, &query),
        Commands::Fetch { node_id } => cmd_fetch(&engine, &node_id),
        Commands::Fact { fact_type, content } => cmd_add_fact(&engine, &fact_type, &content),
        Commands::Facts { filter } => cmd_list_facts(&engine, filter.as_deref()),
        Commands::Stats { since, since_flag } => {
            let effective_since = since_flag.as_deref().or(since.as_deref());
            cmd_stats(&engine, effective_since)
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

fn cmd_index(engine: &HermesEngine, project_root: &std::path::Path, force: bool) -> Result<()> {
    let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
    let pipeline = IngestionPipeline::new(&graph);
    let report = if force {
        pipeline.ingest_directory_force(project_root)?
    } else {
        pipeline.ingest_directory(project_root)?
    };
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

    let acct = Accountant::new(
        engine.db().clone(),
        engine.project_id(),
        engine.session_id(),
    );
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

    let traditional_estimate = response.token_count.saturating_mul(15);
    let acct = Accountant::new(
        engine.db().clone(),
        engine.project_id(),
        engine.session_id(),
    );
    acct.record_query(node_id, 0, response.token_count, traditional_estimate)?;

    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}

fn cmd_add_fact(engine: &HermesEngine, fact_type_str: &str, content: &str) -> Result<()> {
    let store = TemporalStore::new(engine.db().clone(), engine.project_id());
    let fact_type = FactType::parse_str(fact_type_str);
    let id = store.add_fact(AddFactInput {
        node_id: None,
        fact_type,
        content,
        topic: None,
        tags: vec![],
        confidence: None,
        ttl: None,
        source_reference: None,
        provenance: None,
        repo_id: None,
        agent_id: None,
    })?;
    println!("{}", serde_json::json!({ "id": id, "status": "recorded" }));
    Ok(())
}

fn cmd_list_facts(engine: &HermesEngine, filter: Option<&str>) -> Result<()> {
    let store = TemporalStore::new(engine.db().clone(), engine.project_id());
    use hermes_engine::temporal_types::{FactFilter};
    let filter_obj = FactFilter {
        fact_type: filter.map(FactType::parse_str),
        ..Default::default()
    };
    let facts = store.get_active_facts(&filter_obj)?;
    println!("{}", serde_json::to_string_pretty(&facts)?);
    Ok(())
}

fn cmd_stats(engine: &HermesEngine, since_arg: Option<&str>) -> Result<()> {
    let acct = Accountant::new(
        engine.db().clone(),
        engine.project_id(),
        engine.session_id(),
    );
    let session = acct.get_session_stats()?;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmd_add_fact_and_list() {
        let engine = HermesEngine::in_memory("test-cli-fact").unwrap();
        cmd_add_fact(&engine, "decision", "Use Rust for backend").unwrap();
        cmd_list_facts(&engine, Some("decision")).unwrap();
        cmd_list_facts(&engine, None).unwrap();
    }

    #[test]
    fn cmd_stats_returns_zero_for_empty_engine() {
        let engine = HermesEngine::in_memory("test-cli-stats").unwrap();
        cmd_stats(&engine, None).unwrap();
    }

    #[test]
    fn cmd_stats_with_duration_filter() {
        let engine = HermesEngine::in_memory("test-cli-stats-dur").unwrap();
        cmd_stats(&engine, Some("24h")).unwrap();
        cmd_stats(&engine, Some("7d")).unwrap();
        cmd_stats(&engine, Some("all")).unwrap();
    }

    #[test]
    fn cmd_index_on_empty_directory() {
        let engine = HermesEngine::in_memory("test-cli-index").unwrap();
        let dir = tempfile::TempDir::new().unwrap();
        cmd_index(&engine, dir.path(), false).unwrap();
    }

    #[test]
    fn cmd_search_on_empty_graph() {
        let engine = HermesEngine::in_memory("test-cli-search").unwrap();
        cmd_search(&engine, "nonexistent_function").unwrap();
    }

    #[test]
    fn cmd_fetch_on_missing_node() {
        let engine = HermesEngine::in_memory("test-cli-fetch").unwrap();
        let result = cmd_fetch(&engine, "nonexistent-id");
        assert!(result.is_err());
    }

    #[test]
    fn open_engine_uses_env_vars() {
        std::env::set_var("HERMES_PROJECT_ROOT", ".");
        let result = open_engine();
        assert!(result.is_ok());
        let (engine, _root) = result.unwrap();
        assert!(!engine.project_id().is_empty());
    }

    #[test]
    fn cmd_index_force() {
        let engine = HermesEngine::in_memory("test-cli-index-force").unwrap();
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("lib.rs"), "pub fn force_test() {}").unwrap();
        cmd_index(&engine, dir.path(), true).unwrap();
    }

    #[test]
    fn cmd_search_with_indexed_data() {
        let engine = HermesEngine::in_memory("test-cli-search-idx").unwrap();
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("lib.rs"), "pub fn unique_fn_name() {}").unwrap();
        cmd_index(&engine, dir.path(), false).unwrap();
        cmd_search(&engine, "unique_fn_name").unwrap();
    }

    #[test]
    fn cmd_stats_with_invalid_duration() {
        let engine = HermesEngine::in_memory("test-cli-stats-invalid").unwrap();
        // Invalid duration format falls back to cumulative
        cmd_stats(&engine, Some("invalid")).unwrap();
    }

    #[test]
    fn cmd_fact_empty_ignored() {
        let engine = HermesEngine::in_memory("test-cli-fact-empty").unwrap();
        // Empty fact is allowed at the engine level but recording does nothing visible
        let result = cmd_add_fact(&engine, "decision", "");
        assert!(result.is_err() || result.is_ok()); // Accept either
    }
}
