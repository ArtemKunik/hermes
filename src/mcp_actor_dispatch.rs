use anyhow::Result;
use rusqlite::Connection;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{LazyLock, Mutex};

use crate::{mcp_tools, mcp_tools_consistency, mcp_tools_graph, mcp_tools_validation, HermesEngine};

// ── Context & Handler Type ──────────────────────────────────────────────────

/// Bundled dependencies passed to every tool handler.
pub struct ToolContext<'a> {
    pub engine: &'a HermesEngine,
    pub conn: &'a Connection,
    pub project_root: &'a Path,
}

type ToolHandler = fn(&ToolContext, &Value) -> Result<String>;

// ── Registry Infrastructure ─────────────────────────────────────────────────

static TOOL_REGISTRY: LazyLock<Mutex<HashMap<&'static str, ToolHandler>>> = LazyLock::new(|| {
    let mut map = HashMap::new();
    register_tools_inner(&mut map);
    register_missions_inner(&mut map);
    register_memory_inner(&mut map);
    register_compaction_inner(&mut map);
    register_proposals_inner(&mut map);
    register_validation_inner(&mut map);
    register_commit_inner(&mut map);
    register_consistency_inner(&mut map);
    register_status_inner(&mut map);
    register_skills_inner(&mut map);
    register_incidents_inner(&mut map);
    register_tracks_inner(&mut map);
    register_lint_inner(&mut map);
    register_heal_inner(&mut map);
    register_constraints_inner(&mut map);
    register_coverage_inner(&mut map);
    register_quality_inner(&mut map);
    register_dismiss_inner(&mut map);
    register_memory_query_inner(&mut map);
    Mutex::new(map)
});

#[allow(dead_code)]
pub(crate) fn register_tool(name: &'static str, handler: ToolHandler) {
    TOOL_REGISTRY.lock().unwrap().insert(name, handler);
}

fn reg(map: &mut HashMap<&'static str, ToolHandler>, name: &'static str, handler: ToolHandler) {
    map.insert(name, handler);
}

// ── Read-Only Set (kept separate from handler registry for clarity) ─────────

static READ_ONLY_TOOLS: LazyLock<std::collections::HashSet<&'static str>> = LazyLock::new(|| {
    [
        "hermes_search",
        "hermes_fetch",
        "hermes_stats",
        "hermes_slow_loop_status",
        "hermes_facts",
        "hermes_memory_stats",
        "hermes_recall",
        "hermes_validate_env",
        "hermes_validate_symbols",
        "hermes_repo_map",
        "hermes_check_consistency",
        "hermes_impact_analysis",
        "hermes_mcp_status",
        "hermes_tools",
        "hermes_match_skills",
        "hermes_fetch_skill",
        "hermes_query_incidents",
        "hermes_list_tracks",
        "hermes_resume_track",
        "hermes_search_kb",
        "hermes_constraints",
        "hermes_test_coverage_map",
        "hermes_search_misses",
        "hermes_query_memory",
        "hermes_get_core_facts",
        "hermes_blast_score",
        "hermes_high_blast",
        "hermes_lookup",
        "hermes_file_symbols",
        "hermes_proposal_list",
        "hermes_validate_commit_context",
    ]
    .into_iter()
    .collect()
});

pub(crate) fn is_read_only_tool(name: &str) -> bool {
    READ_ONLY_TOOLS.contains(name)
}

// ── Dispatch ────────────────────────────────────────────────────────────────

pub(crate) fn execute_tool_call(
    engine: &HermesEngine,
    conn: &Connection,
    project_root: &Path,
    name: &str,
    args: &Value,
) -> Result<String> {
    let ctx = ToolContext {
        engine,
        conn,
        project_root,
    };
    let handlers = TOOL_REGISTRY.lock().unwrap();
    let handler = handlers
        .get(name)
        .ok_or_else(|| anyhow::anyhow!("unknown tool: {name}"))?;
    handler(&ctx, args)
}

// ── Tool Registrations ─────────────────────────────────────────────────────

/// Tools from `mcp_tools` module.
fn register_tools_inner(map: &mut HashMap<&'static str, ToolHandler>) {
    use crate::mcp_tools;

    reg(map, "hermes_search", |ctx, args| {
        let query = args["query"].as_str().unwrap_or("");
        anyhow::ensure!(!query.is_empty(), "hermes_search requires 'query'");
        let goal = args["goal"].as_str();
        mcp_tools::tool_search(ctx.engine, query, goal)
    });

    reg(map, "hermes_fetch", |ctx, args| {
        let node_id = args["node_id"].as_str().unwrap_or("");
        anyhow::ensure!(!node_id.is_empty(), "hermes_fetch requires 'node_id'");
        mcp_tools::tool_fetch(ctx.engine, node_id)
    });

    reg(map, "hermes_lookup", |ctx, args| {
        let symbol_name = args["symbol_name"].as_str().unwrap_or("");
        anyhow::ensure!(
            !symbol_name.is_empty(),
            "hermes_lookup requires 'symbol_name'"
        );
        mcp_tools::tool_lookup(ctx.engine, symbol_name)
    });

    reg(map, "hermes_file_symbols", |ctx, args| {
        let file_path = args["file_path"].as_str().unwrap_or("");
        anyhow::ensure!(
            !file_path.is_empty(),
            "hermes_file_symbols requires 'file_path'"
        );
        mcp_tools::tool_file_symbols(ctx.engine, file_path)
    });

    reg(map, "hermes_index", |ctx, _| {
        mcp_tools::tool_index(ctx.engine, ctx.project_root)
    });

    reg(map, "hermes_backfill", |ctx, _| {
        mcp_tools::tool_backfill(ctx.engine)
    });

    reg(map, "hermes_stats", |ctx, _| {
        mcp_tools::tool_stats_with_conn(ctx.engine, ctx.conn)
    });

    reg(map, "hermes_slow_loop_status", |ctx, _| {
        mcp_tools::tool_slow_loop_status(ctx.engine)
    });

    reg(map, "hermes_generate_digest", |ctx, args| {
        let date = args["date"].as_str().unwrap_or("");
        anyhow::ensure!(!date.is_empty(), "hermes_generate_digest requires 'date'");
        mcp_tools::tool_generate_digest(ctx.engine, date)
    });

    reg(map, "hermes_compact_sessions", |ctx, _| {
        mcp_tools::tool_compact_sessions(ctx.engine)
    });

    reg(map, "hermes_generate_weekly_brief", |ctx, _| {
        mcp_tools::tool_generate_weekly_brief(ctx.engine)
    });

    reg(map, "hermes_approve_skill_candidate", |ctx, args| {
        let name = args["name"].as_str().unwrap_or("");
        anyhow::ensure!(
            !name.is_empty(),
            "hermes_approve_skill_candidate requires 'name'"
        );
        mcp_tools::tool_approve_skill_candidate(ctx.engine, name)
    });

    reg(map, "hermes_reject_skill_candidate", |ctx, args| {
        let name = args["name"].as_str().unwrap_or("");
        anyhow::ensure!(
            !name.is_empty(),
            "hermes_reject_skill_candidate requires 'name'"
        );
        mcp_tools::tool_reject_skill_candidate(ctx.engine, name)
    });

    reg(map, "hermes_apply_proposal", |ctx, args| {
        let filename = args["filename"].as_str().unwrap_or("");
        anyhow::ensure!(
            !filename.is_empty(),
            "hermes_apply_proposal requires 'filename'"
        );
        mcp_tools::tool_apply_proposal(ctx.engine, filename)
    });

    reg(map, "hermes_fact", |ctx, args| {
        let ft = args["fact_type"].as_str().unwrap_or("observation");
        let c = args["content"].as_str().unwrap_or("");
        anyhow::ensure!(!c.is_empty(), "hermes_fact requires 'content'");
        let node_id = args["node_id"].as_str();
        let topic = args["topic"].as_str();
        let tags: Vec<String> = args["tags"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();
        let confidence = args["confidence"].as_f64();
        let ttl = args["ttl"].as_str();
        let source_reference = args["source_reference"].as_str();
        let provenance = args["provenance"].as_str();
        let repo_id = args["repo_id"].as_str();
        let agent_id = args["agent_id"].as_str();
        mcp_tools::tool_add_fact_with_conn_full(
            ctx.engine,
            ctx.conn,
            ft,
            c,
            node_id,
            topic,
            &tags,
            confidence,
            ttl,
            source_reference,
            provenance,
            repo_id,
            agent_id,
        )
    });

    reg(map, "hermes_facts", |ctx, args| {
        let filter = args["fact_type"].as_str();
        mcp_tools::tool_list_facts(ctx.engine, filter)
    });

    reg(map, "hermes_fact_expire", |ctx, args| {
        let fact_id = args["fact_id"].as_str().unwrap_or("");
        anyhow::ensure!(!fact_id.is_empty(), "hermes_fact_expire requires 'fact_id'");
        let superseded_by = args["superseded_by"].as_str();
        mcp_tools::tool_expire_fact_with_conn(ctx.engine, ctx.conn, fact_id, superseded_by)
    });

    reg(map, "hermes_repo_map", |ctx, args| {
        let max_tokens = args["max_tokens"].as_u64().unwrap_or(2048) as usize;
        mcp_tools::tool_repo_map(ctx.engine, max_tokens)
    });

    reg(map, "hermes_impact_analysis", |ctx, args| {
        let symbol = args["symbol_name"].as_str().unwrap_or("");
        anyhow::ensure!(
            !symbol.is_empty(),
            "hermes_impact_analysis requires 'symbol_name'"
        );
        mcp_tools::tool_impact_analysis(ctx.engine, symbol)
    });

    reg(map, "hermes_blast_score", |ctx, args| {
        let query = args["query"].as_str().unwrap_or("");
        anyhow::ensure!(!query.is_empty(), "hermes_blast_score requires 'query'");
        mcp_tools::tool_blast_score(ctx.engine, query)
    });

    reg(map, "hermes_high_blast", |ctx, args| {
        let threshold = args["threshold"].as_f64().unwrap_or(1.0);
        let limit = args["limit"].as_u64().unwrap_or(20) as usize;
        mcp_tools::tool_high_blast(ctx.engine, threshold, limit)
    });

    reg(map, "hermes_graph", |ctx, args| {
        let node_ids: Option<Vec<String>> = args["node_ids"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect());
        let node_types: Option<Vec<String>> = args["node_types"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect());
        let edge_types: Option<Vec<String>> = args["edge_types"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect());
        let limit = args["limit"].as_u64().map(|n| n as usize);
        mcp_tools_graph::tool_graph(ctx.engine, node_ids, node_types, edge_types, limit)
    });

    reg(map, "hermes_neighbors", |ctx, args| {
        let node_id = args["node_id"].as_str().unwrap_or("");
        anyhow::ensure!(!node_id.is_empty(), "hermes_neighbors requires 'node_id'");
        let edge_types: Option<Vec<String>> = args["edge_types"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect());
        let limit = args["limit"].as_u64().map(|n| n as usize);
        mcp_tools_graph::tool_neighbors(ctx.engine, node_id, edge_types, limit)
    });

    reg(map, "hermes_search_misses", |ctx, args| {
        let since_days = args["since_days"].as_u64();
        let top_k = args["top_k"].as_u64().unwrap_or(10) as usize;
        mcp_tools::tool_search_misses(ctx.engine, since_days, top_k)
    });
}

/// Tools from `mcp_missions` module.
fn register_missions_inner(map: &mut HashMap<&'static str, ToolHandler>) {
    use crate::mcp_missions;

    reg(map, "hermes_mission_start", |ctx, args| {
        mcp_missions::tool_mission_start(ctx.engine, ctx.conn, args)
    });
    reg(map, "hermes_mission_update", |ctx, args| {
        mcp_missions::tool_mission_update(ctx.engine, ctx.conn, args)
    });
    reg(map, "hermes_mission_event", |ctx, args| {
        mcp_missions::tool_mission_event(ctx.engine, ctx.conn, args)
    });
    reg(map, "hermes_mission_status", |ctx, args| {
        mcp_missions::tool_mission_status(ctx.engine, ctx.conn, args)
    });
    reg(map, "hermes_mission_list", |ctx, args| {
        mcp_missions::tool_mission_list(ctx.engine, ctx.conn, args)
    });
    reg(map, "hermes_mission_heartbeat", |ctx, args| {
        mcp_missions::tool_mission_heartbeat(ctx.engine, ctx.conn, args)
    });
}

/// Tools from `mcp_memory` module.
fn register_memory_inner(map: &mut HashMap<&'static str, ToolHandler>) {
    use crate::mcp_memory;

    reg(map, "hermes_remember", |ctx, args| {
        mcp_memory::tool_remember(ctx.engine, ctx.project_root, args, ctx.conn)
    });
    reg(map, "hermes_write_decision", |ctx, args| {
        anyhow::ensure!(
            args["title"].as_str().is_some_and(|t| !t.is_empty()),
            "hermes_write_decision requires 'title'"
        );
        mcp_memory::tool_write_decision(ctx.engine, ctx.project_root, args, ctx.conn)
    });
    reg(map, "hermes_memory_stats", |ctx, _| {
        mcp_memory::tool_memory_stats_with_conn(ctx.engine, ctx.conn)
    });
    reg(map, "hermes_battery_check", |ctx, args| {
        mcp_memory::tool_battery_check(ctx.engine, args)
    });
    reg(map, "hermes_recall", |ctx, args| {
        let query = args["query"]
            .as_str()
            .or_else(|| args["topic"].as_str())
            .unwrap_or("");
        anyhow::ensure!(
            !query.is_empty(),
            "hermes_recall requires 'query' or 'topic'"
        );
        mcp_memory::tool_recall_with_conn(ctx.engine, ctx.conn, query)
    });
}

/// Tools from `mcp_compaction` module.
fn register_compaction_inner(map: &mut HashMap<&'static str, ToolHandler>) {
    use crate::mcp_compaction;

    reg(map, "hermes_compact_session", |ctx, args| {
        mcp_compaction::tool_compact_session(ctx.engine, ctx.project_root, args)
    });
}

/// Tools from `mcp_proposals` module.
fn register_proposals_inner(map: &mut HashMap<&'static str, ToolHandler>) {
    use crate::mcp_proposals;

    reg(map, "hermes_proposal_create", |ctx, args| {
        mcp_proposals::tool_proposal_create(ctx.engine, ctx.conn, args)
    });
    reg(map, "hermes_proposal_list", |ctx, args| {
        mcp_proposals::tool_proposal_list(ctx.engine, ctx.conn, args)
    });
    reg(map, "hermes_proposal_update", |ctx, args| {
        mcp_proposals::tool_proposal_update(ctx.engine, ctx.conn, args)
    });
    reg(map, "hermes_proposal_reject", |ctx, args| {
        mcp_proposals::tool_proposal_reject(ctx.engine, ctx.conn, args)
    });
    reg(map, "hermes_proposal_approve", |ctx, args| {
        mcp_proposals::tool_proposal_approve(ctx.engine, ctx.conn, args)
    });
}

/// Tools from `mcp_tools_validation` module.
fn register_validation_inner(map: &mut HashMap<&'static str, ToolHandler>) {
    use crate::mcp_tools_validation;

    reg(map, "hermes_validate_env", |ctx, args| {
        let env_var = args["env_var"].as_str().unwrap_or("");
        anyhow::ensure!(
            !env_var.is_empty(),
            "hermes_validate_env requires 'env_var'"
        );
        mcp_tools_validation::tool_validate_env_with_conn(ctx.engine, ctx.conn, env_var)
    });

    reg(map, "hermes_validate_symbols", |ctx, args| {
        let symbols: Vec<&str> = args["symbols"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();
        anyhow::ensure!(
            !symbols.is_empty(),
            "hermes_validate_symbols requires 'symbols'"
        );
        mcp_tools_validation::tool_validate_symbols_with_conn(ctx.engine, ctx.conn, &symbols)
    });
}

/// Tools from `mcp_commit` module.
fn register_commit_inner(map: &mut HashMap<&'static str, ToolHandler>) {
    reg(map, "hermes_prepare_commit_message", |_, args| {
        crate::mcp_commit::tool_prepare_commit_message(args)
    });

    reg(map, "hermes_validate_commit_context", |_, args| {
        let message = args["message"].as_str().unwrap_or("");
        anyhow::ensure!(
            !message.is_empty(),
            "hermes_validate_commit_context requires 'message'"
        );
        crate::mcp_commit::tool_validate_commit_context(message)
    });
}

/// Tools from `mcp_tools_consistency` module.
fn register_consistency_inner(map: &mut HashMap<&'static str, ToolHandler>) {
    use crate::mcp_tools_consistency;

    reg(map, "hermes_check_consistency", |ctx, _| {
        mcp_tools_consistency::tool_check_consistency_with_conn(ctx.engine, ctx.conn)
    });
}

/// Tools from `mcp_status` module.
fn register_status_inner(map: &mut HashMap<&'static str, ToolHandler>) {
    use crate::mcp_status;

    reg(map, "hermes_mcp_status", |ctx, _| {
        let val = mcp_status::tool_mcp_status(ctx.engine, ctx.project_root)?;
        Ok(serde_json::to_string_pretty(&val)?)
    });
}

/// Tools from `mcp_skills` module.
fn register_skills_inner(map: &mut HashMap<&'static str, ToolHandler>) {
    use crate::mcp_skills;

    reg(map, "hermes_tools", |_, args| {
        let intent = args["intent"].as_str().unwrap_or("all");
        let val = crate::tool_router::tools_payload_for_intent(intent);
        Ok(serde_json::to_string_pretty(&val)?)
    });

    reg(map, "hermes_match_skills", |ctx, args| {
        let query = args["query"].as_str().unwrap_or("");
        anyhow::ensure!(!query.is_empty(), "hermes_match_skills requires 'query'");
        let scope = args["scope"].as_str();
        let val = mcp_skills::tool_match_skills_with_conn(ctx.engine, ctx.conn, query, scope)?;
        Ok(serde_json::to_string_pretty(&val)?)
    });

    reg(map, "hermes_fetch_skill", |ctx, args| {
        let skill_path = args["skill_path"].as_str().unwrap_or("");
        anyhow::ensure!(
            !skill_path.is_empty(),
            "hermes_fetch_skill requires 'skill_path'"
        );
        let val = mcp_skills::tool_fetch_skill_with_conn(ctx.engine, ctx.conn, skill_path)?;
        Ok(serde_json::to_string_pretty(&val)?)
    });
}

/// Tools from `mcp_incidents` module.
fn register_incidents_inner(map: &mut HashMap<&'static str, ToolHandler>) {
    use crate::mcp_incidents;

    reg(map, "hermes_log_incident", |ctx, args| {
        anyhow::ensure!(
            args["title"].as_str().is_some_and(|t| !t.is_empty()),
            "hermes_log_incident requires 'title'"
        );
        mcp_incidents::tool_log_incident(ctx.engine, ctx.project_root, args)
    });

    reg(map, "hermes_resolve_incident", |ctx, args| {
        anyhow::ensure!(
            args["sub_product"].as_str().is_some() && args["slug"].as_str().is_some(),
            "hermes_resolve_incident requires 'sub_product' and 'slug'"
        );
        mcp_incidents::tool_resolve_incident(ctx.engine, ctx.project_root, args)
    });

    reg(map, "hermes_query_incidents", |ctx, args| {
        mcp_incidents::tool_query_incidents_with_conn(ctx.engine, ctx.conn, ctx.project_root, args)
    });

    reg(map, "hermes_write_kb_article", |ctx, args| {
        anyhow::ensure!(
            args["title"].as_str().is_some_and(|t| !t.is_empty()),
            "hermes_write_kb_article requires 'title'"
        );
        mcp_incidents::tool_write_kb_article(ctx.engine, ctx.project_root, args)
    });

    reg(map, "hermes_search_kb", |ctx, args| {
        anyhow::ensure!(
            args["query"].as_str().is_some_and(|q| !q.is_empty()),
            "hermes_search_kb requires 'query'"
        );
        mcp_incidents::tool_search_kb_with_conn(ctx.engine, ctx.conn, args)
    });
}

/// Tools from `mcp_tracks` module.
fn register_tracks_inner(map: &mut HashMap<&'static str, ToolHandler>) {
    use crate::mcp_tracks;

    reg(map, "hermes_list_tracks", |ctx, args| {
        mcp_tracks::tool_list_tracks_with_conn(ctx.engine, ctx.conn, ctx.project_root, args)
    });

    reg(map, "hermes_resume_track", |ctx, args| {
        mcp_tracks::tool_resume_track_with_conn(ctx.engine, ctx.conn, ctx.project_root, args)
    });
}

/// Tools from `mcp_lint` module.
fn register_lint_inner(map: &mut HashMap<&'static str, ToolHandler>) {
    use crate::mcp_lint;

    reg(map, "hermes_lint_architecture", |ctx, args| {
        mcp_lint::tool_lint_architecture(ctx.engine, ctx.project_root, args)
    });
}

/// Tools from `mcp_heal` module.
fn register_heal_inner(map: &mut HashMap<&'static str, ToolHandler>) {
    use crate::mcp_heal;

    reg(map, "hermes_heal_violations", |ctx, args| {
        mcp_heal::tool_heal_violations(ctx.engine, ctx.project_root, args)
    });
}

/// Tools from `mcp_constraints` module.
fn register_constraints_inner(map: &mut HashMap<&'static str, ToolHandler>) {
    use crate::mcp_constraints;

    reg(map, "hermes_constraints", |ctx, args| {
        let file_path = args["file_path"].as_str().unwrap_or("");
        anyhow::ensure!(
            !file_path.is_empty(),
            "hermes_constraints requires 'file_path'"
        );
        mcp_constraints::tool_constraints(ctx.engine, ctx.project_root, args)
    });
}

/// Tools from `mcp_coverage` module.
fn register_coverage_inner(map: &mut HashMap<&'static str, ToolHandler>) {
    use crate::mcp_coverage;

    reg(map, "hermes_test_coverage_map", |ctx, args| {
        mcp_coverage::tool_test_coverage_map_with_conn(ctx.engine, ctx.conn, ctx.project_root, args)
    });
}

/// Tools from `mcp_quality` module.
fn register_quality_inner(map: &mut HashMap<&'static str, ToolHandler>) {
    use crate::mcp_quality;

    reg(map, "hermes_quality_review", |ctx, args| {
        mcp_quality::tool_quality_review(ctx.engine, ctx.project_root, args)
    });
    reg(map, "hermes_quality_score", |ctx, args| {
        mcp_quality::tool_quality_score(ctx.engine, ctx.project_root, args)
    });
    reg(map, "hermes_quality_next", |ctx, args| {
        mcp_quality::tool_quality_next(ctx.engine, ctx.project_root, args)
    });
    reg(map, "hermes_quality_resolve", |ctx, args| {
        mcp_quality::tool_quality_resolve(ctx.engine, ctx.project_root, args)
    });
    reg(map, "hermes_quality_wontfix", |ctx, args| {
        mcp_quality::tool_quality_wontfix(ctx.engine, ctx.project_root, args)
    });
}

/// Tools from `mcp_dismiss` module.
fn register_dismiss_inner(map: &mut HashMap<&'static str, ToolHandler>) {
    use crate::mcp_dismiss;

    reg(map, "hermes_quality_dismiss", |ctx, args| {
        mcp_dismiss::tool_quality_dismiss(ctx.engine, ctx.project_root, args)
    });
    reg(map, "hermes_lint_dismiss", |ctx, args| {
        mcp_dismiss::tool_lint_dismiss(ctx.engine, ctx.project_root, args)
    });
    reg(map, "hermes_dismissed_list", |ctx, args| {
        mcp_dismiss::tool_dismissed_list(ctx.engine, ctx.project_root, args)
    });
    reg(map, "hermes_auto_dismiss", |ctx, args| {
        mcp_dismiss::tool_auto_dismiss(ctx.engine, ctx.project_root, args)
    });
}

/// Tools from `memory_query` module.
fn register_memory_query_inner(map: &mut HashMap<&'static str, ToolHandler>) {
    use crate::memory_query;

    reg(map, "hermes_query_memory", |ctx, args| {
        let query = args["query"].as_str().unwrap_or("");
        anyhow::ensure!(!query.is_empty(), "hermes_query_memory requires 'query'");
        let limit = args["limit"]
            .as_u64()
            .map(|n| n as usize)
            .unwrap_or(crate::memory_query::DEFAULT_QUERY_LIMIT);
        memory_query::tool_query_memory_with_conn(ctx.conn, query, limit)
    });

    reg(map, "hermes_get_core_facts", |ctx, _| {
        memory_query::tool_get_core_facts_with_conn(ctx.conn, ctx.project_root)
    });
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn unknown_tool_returns_error() {
        let engine = HermesEngine::in_memory("test").unwrap();
        let conn = engine.db().lock().unwrap();
        let project_root = std::path::PathBuf::new();
        let result = execute_tool_call(
            &engine,
            &conn,
            &project_root,
            "hermes_nonexistent",
            &json!({}),
        );
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("unknown tool"),
            "expected 'unknown tool' error, got: {err}"
        );
    }

    #[test]
    fn read_only_tools_detected() {
        assert!(is_read_only_tool("hermes_search"));
        assert!(is_read_only_tool("hermes_stats"));
        assert!(is_read_only_tool("hermes_query_memory"));
        assert!(!is_read_only_tool("hermes_index"));
        assert!(!is_read_only_tool("hermes_backfill"));
    }

    #[test]
    fn registry_contains_tools() {
        let handlers = TOOL_REGISTRY.lock().unwrap();
        // Spot-check a few tools are registered
        assert!(handlers.contains_key("hermes_search"));
        assert!(handlers.contains_key("hermes_index"));
        assert!(handlers.contains_key("hermes_mission_start"));
        assert!(handlers.contains_key("hermes_quality_review"));
    }

    #[test]
    fn registry_has_expected_count() {
        let handlers = TOOL_REGISTRY.lock().unwrap();
        assert_eq!(handlers.len(), 69, "expected 69 tools in registry");
    }
}
