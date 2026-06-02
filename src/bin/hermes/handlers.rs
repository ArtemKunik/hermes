// tools/hermes-engine/src/bin/hermes/handlers.rs
use anyhow::{bail, Result};
use hermes_engine::mcp_memory;
use hermes_engine::{
    accounting::{parse_since_duration, Accountant},
    graph::KnowledgeGraph,
    ingestion::{llm_enricher::LlmEnricher, IngestionPipeline},
    search::{SearchEngine, SearchMode},
    temporal::{FactType, TemporalStore},
    viz,
    weight::WeightStore,
    HermesEngine,
};
use std::path::Path;

pub fn cmd_viz(engine: &HermesEngine, project_root: &Path, port: u16) -> Result<()> {
    viz::server::run_viz_server(engine, project_root, port)
}

pub fn cmd_index(engine: &HermesEngine, project_root: &Path, enrich: bool) -> Result<()> {
    let _index_lock = match hermes_engine::index_lock::try_acquire_index_lock(project_root)? {
        hermes_engine::index_lock::LockAcquisition::Acquired(lock) => lock,
        hermes_engine::index_lock::LockAcquisition::Busy(_) => {
            print_non_blocking_busy_index("index already running");
            return Ok(());
        }
    };

    let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
    let mut pipeline = IngestionPipeline::new(&graph);
    if enrich {
        pipeline = pipeline.with_enricher(LlmEnricher::from_env());
        tracing::info!(
            "LLM enrichment enabled via {}",
            std::env::var("HERMES_LLM_GATEWAY_URL")
                .unwrap_or_else(|_| "http://localhost:3001".to_string())
        );
    }
    let report = match pipeline.ingest_directory(project_root) {
        Ok(report) => report,
        Err(err) if hermes_engine::retry::is_database_locked_message(&err.to_string()) => {
            print_non_blocking_busy_index("database is locked");
            return Ok(());
        }
        Err(err) => return Err(err),
    };
    // Task 1.3: Invalidate search cache so stale results are not returned
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

pub fn print_non_blocking_busy_index(reason: &str) {
    let output = serde_json::json!({
        "status": "busy",
        "reason": reason,
        "non_blocking": true,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string())
    );
}

pub fn cmd_search(engine: &HermesEngine, query: &str) -> Result<()> {
    let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
    let search = SearchEngine::new(&graph, engine.search_cache());
    let response = search.search(query, 10, &SearchMode::Smart)?;

    // Record to accounting: pointer tokens used, 0 tokens fetched (search only)
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

    // Persist zero-result searches for post-mortem analysis.
    if response.pointers.is_empty() {
        let _ = acct.record_search_miss(query, None, None, "cli");
    }

    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}

pub fn cmd_fetch(engine: &HermesEngine, node_id: &str) -> Result<()> {
    let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
    let search = SearchEngine::new(&graph, engine.search_cache());

    let Some(response) = search.fetch(node_id)? else {
        bail!("node not found: {node_id}");
    };

    // Record to accounting: 0 pointer tokens, actual fetched tokens, with traditional estimate
    // Traditional estimate: respect feature flag which may prefer stored node token counts.
    let traditional_estimate = if std::env::var("HERMES_ENABLE_REAL_TRADITIONAL_ESTIMATE")
        .map(|v| v != "0" && v.to_lowercase() != "false")
        .unwrap_or(false)
    {
        response.content_tokens.unwrap_or(response.token_count)
    } else {
        response.token_count.saturating_mul(15)
    };
    let acct = Accountant::new(
        engine.db().clone(),
        engine.project_id(),
        engine.session_id(),
    );
    acct.record_query(node_id, 0, response.token_count, traditional_estimate)?;

    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}

pub fn cmd_list_tracks(engine: &HermesEngine, project_root: &Path, status: Option<&str>) -> Result<()> {
    let args = serde_json::json!({ "status": status.unwrap_or("unfinished") });
    println!("{}", hermes_engine::mcp_tracks::tool_list_tracks(engine, project_root, &args)?);
    Ok(())
}

pub fn cmd_resume_track(engine: &HermesEngine, project_root: &Path, track_id: Option<&str>, auto: bool, status: Option<&str>) -> Result<()> {
    let args = serde_json::json!({
        "track_id": track_id.unwrap_or(""),
        "auto": auto,
        "status": status.unwrap_or("unfinished")
    });
    println!("{}", hermes_engine::mcp_tracks::tool_resume_track(engine, project_root, &args)?);
    Ok(())
}

pub fn cmd_recall(engine: &HermesEngine, query: &str) -> Result<()> {
    let response = mcp_memory::tool_recall(engine, query)?;
    println!("{response}");
    Ok(())
}

pub fn cmd_add_fact(engine: &HermesEngine, fact_type_str: &str, content: &str) -> Result<()> {
    use hermes_engine::temporal::AddFactInput;
    let store = TemporalStore::new(engine.db().clone(), engine.project_id());
    let id = store.add_fact(AddFactInput {
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
    })?;
    println!("{}", serde_json::json!({ "id": id, "status": "recorded" }));
    Ok(())
}

pub fn cmd_list_facts(engine: &HermesEngine, filter: Option<&str>) -> Result<()> {
    use hermes_engine::temporal::FactFilter;
    let store = TemporalStore::new(engine.db().clone(), engine.project_id());
    let f = FactFilter {
        fact_type: filter.map(FactType::parse_str),
        ..Default::default()
    };
    let facts = store.get_active_facts(&f)?;
    println!("{}", serde_json::to_string_pretty(&facts)?);
    Ok(())
}

pub fn cmd_stats(engine: &HermesEngine, since_arg: Option<&str>) -> Result<()> {
    let acct = Accountant::new(
        engine.db().clone(),
        engine.project_id(),
        engine.session_id(),
    );
    let session = acct.get_session_stats()?;

    // Task 2.3: Apply --since filter to cumulative stats when specified
    let since_dur = since_arg.and_then(parse_since_duration);
    let cumulative = acct.get_stats_since(since_dur)?;
    let by_tool = acct
        .get_stats_by_tool(since_dur)?
        .into_iter()
        .map(|(tool, queries, saved)| {
            serde_json::json!({"tool": tool, "queries": queries, "tokens_saved": saved})
        })
        .collect::<Vec<_>>();
    let impact = acct.get_impact_summary()?;

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
        "by_tool": by_tool,
        "impact": impact,
    });
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

pub fn cmd_weight_get(engine: &HermesEngine, node_id: &str) -> Result<()> {
    let store = WeightStore::new(engine.db().clone());
    match store.get_record(node_id)? {
        Some(r) => println!("{}", serde_json::to_string_pretty(&r)?),
        None => println!(
            "{}",
            serde_json::json!({
                "node_id": node_id,
                "weight": 1.0,
                "reinforcement_count": 0,
                "decay_count": 0,
                "last_updated": null
            })
        ),
    }
    Ok(())
}

pub fn cmd_weight_set(engine: &HermesEngine, node_id: &str, delta: f64) -> Result<()> {
    let store = WeightStore::new(engine.db().clone());
    let record = store.adjust_weight(node_id, delta)?;
    println!("{}", serde_json::to_string_pretty(&record)?);
    Ok(())
}

pub fn cmd_weight_list(engine: &HermesEngine) -> Result<()> {
    let store = WeightStore::new(engine.db().clone());
    let records = store.list_non_default()?;
    println!("{}", serde_json::to_string_pretty(&records)?);
    Ok(())
}

/// List ALL graph nodes with their weights (default 1.0 for nodes never explicitly adjusted).
/// This is the correct source for AD-04 consolidation — weight-list alone misses every node
/// that has never been reinforced or decayed.
pub fn cmd_nodes_weight_list(engine: &HermesEngine) -> Result<()> {
    let store = WeightStore::new(engine.db().clone());
    let records = store.list_all_nodes_with_weights(engine.project_id())?;
    println!("{}", serde_json::to_string_pretty(&records)?);
    Ok(())
}

pub fn cmd_delete_node(engine: &HermesEngine, node_id: &str) -> Result<()> {
    let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
    // Verify the node exists before attempting deletion.
    if graph.get_node(node_id)?.is_none() {
        bail!("node not found: {node_id}");
    }
    graph.delete_node(node_id)?;
    println!(
        "{}",
        serde_json::json!({ "node_id": node_id, "status": "deleted" })
    );
    Ok(())
}

pub fn cmd_backfill(engine: &HermesEngine) -> Result<()> {
    let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
    let pipeline = IngestionPipeline::new(&graph);
    let updated = pipeline.backfill_content_tokens()?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({ "nodes_updated": updated }))?
    );
    Ok(())
}

