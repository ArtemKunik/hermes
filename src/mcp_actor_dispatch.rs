use anyhow::Result;
use rusqlite::Connection;
use serde_json::Value;
use std::path::Path;

use crate::{mcp_tools, mcp_tools_consistency, mcp_tools_validation, HermesEngine};

pub(crate) fn is_read_only_tool(name: &str) -> bool {
    matches!(
        name,
        "hermes_search"
            | "hermes_fetch"
            | "hermes_stats"
            | "hermes_slow_loop_status"
            | "hermes_facts"
            | "hermes_memory_stats"
            | "hermes_recall"
            | "hermes_validate_env"
            | "hermes_validate_symbols"
            | "hermes_repo_map"
            | "hermes_check_consistency"
            | "hermes_impact_analysis"
            | "hermes_mcp_status"
            | "hermes_tools"
            | "hermes_match_skills"
            | "hermes_fetch_skill"
            | "hermes_query_incidents"
            | "hermes_list_tracks"
            | "hermes_resume_track"
            | "hermes_search_kb"
            | "hermes_constraints"
            | "hermes_test_coverage_map"
            | "hermes_search_misses"
            | "hermes_query_memory"
            | "hermes_get_core_facts"
    )
}

pub(crate) fn execute_tool_call(
    engine: &HermesEngine,
    conn: &Connection,
    project_root: &Path,
    name: &str,
    args: &Value,
) -> Result<String> {
    match name {
        "hermes_search" => {
            let query = args["query"].as_str().unwrap_or("");
            anyhow::ensure!(!query.is_empty(), "hermes_search requires 'query'");
            let goal = args["goal"].as_str();
            let _ = conn;
            mcp_tools::tool_search(engine, query, goal)
        }
        "hermes_fetch" => {
            let node_id = args["node_id"].as_str().unwrap_or("");
            anyhow::ensure!(!node_id.is_empty(), "hermes_fetch requires 'node_id'");
            let _ = conn;
            mcp_tools::tool_fetch(engine, node_id)
        }
        "hermes_index" => mcp_tools::tool_index(engine, project_root),
        "hermes_backfill" => mcp_tools::tool_backfill(engine),
        "hermes_stats" => mcp_tools::tool_stats_with_conn(engine, conn),
        "hermes_slow_loop_status" => mcp_tools::tool_slow_loop_status(engine),
        "hermes_generate_digest" => {
            let date = args["date"].as_str().unwrap_or("");
            anyhow::ensure!(!date.is_empty(), "hermes_generate_digest requires 'date'");
            mcp_tools::tool_generate_digest(engine, date)
        }
        "hermes_compact_sessions" => mcp_tools::tool_compact_sessions(engine),
        "hermes_generate_weekly_brief" => mcp_tools::tool_generate_weekly_brief(engine),
        "hermes_approve_skill_candidate" => {
            let name = args["name"].as_str().unwrap_or("");
            anyhow::ensure!(!name.is_empty(), "hermes_approve_skill_candidate requires 'name'");
            mcp_tools::tool_approve_skill_candidate(engine, name)
        }
        "hermes_reject_skill_candidate" => {
            let name = args["name"].as_str().unwrap_or("");
            anyhow::ensure!(!name.is_empty(), "hermes_reject_skill_candidate requires 'name'");
            mcp_tools::tool_reject_skill_candidate(engine, name)
        }
        "hermes_apply_proposal" => {
            let filename = args["filename"].as_str().unwrap_or("");
            anyhow::ensure!(!filename.is_empty(), "hermes_apply_proposal requires 'filename'");
            mcp_tools::tool_apply_proposal(engine, filename)
        }
        "hermes_fact" => {
            let ft = args["fact_type"].as_str().unwrap_or("");
            let c = args["content"].as_str().unwrap_or("");
            anyhow::ensure!(!ft.is_empty() && !c.is_empty(), "hermes_fact requires 'fact_type' and 'content'");
            mcp_tools::tool_add_fact_with_conn(engine, conn, ft, c)
        }
        "hermes_facts" => {
            let filter = args["fact_type"].as_str();
            let _ = conn;
            mcp_tools::tool_list_facts(engine, filter)
        }
        "hermes_mission_start" => crate::mcp_missions::tool_mission_start(engine, conn, args),
        "hermes_mission_update" => crate::mcp_missions::tool_mission_update(engine, conn, args),
        "hermes_mission_event" => crate::mcp_missions::tool_mission_event(engine, conn, args),
        "hermes_mission_status" => crate::mcp_missions::tool_mission_status(engine, conn, args),
        "hermes_mission_list" => crate::mcp_missions::tool_mission_list(engine, conn, args),
        "hermes_remember" => crate::mcp_memory::tool_remember(engine, project_root, args, conn),
        "hermes_compact_session" => crate::mcp_compaction::tool_compact_session(engine, project_root, args),
        "hermes_write_decision" => {
            anyhow::ensure!(
                args["title"].as_str().is_some_and(|t| !t.is_empty()),
                "hermes_write_decision requires 'title'"
            );
            crate::mcp_memory::tool_write_decision(engine, project_root, args, conn)
        }
        "hermes_memory_stats" => crate::mcp_memory::tool_memory_stats_with_conn(engine, conn),
        "hermes_battery_check" => crate::mcp_memory::tool_battery_check(engine, args),
        "hermes_recall" => {
            let query = args["query"].as_str().or_else(|| args["topic"].as_str()).unwrap_or("");
            anyhow::ensure!(!query.is_empty(), "hermes_recall requires 'query' or 'topic'");
            crate::mcp_memory::tool_recall_with_conn(engine, conn, query)
        }
        "hermes_validate_env" => {
            let env_var = args["env_var"].as_str().unwrap_or("");
            anyhow::ensure!(!env_var.is_empty(), "hermes_validate_env requires 'env_var'");
            mcp_tools_validation::tool_validate_env_with_conn(engine, conn, env_var)
        }
        "hermes_validate_symbols" => {
            let symbols: Vec<&str> = args["symbols"]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();
            anyhow::ensure!(!symbols.is_empty(), "hermes_validate_symbols requires 'symbols'");
            mcp_tools_validation::tool_validate_symbols_with_conn(engine, conn, &symbols)
        }
        "hermes_prepare_commit_message" => crate::mcp_commit::tool_prepare_commit_message(args),
        "hermes_repo_map" => {
            let max_tokens = args["max_tokens"].as_u64().unwrap_or(2048) as usize;
            let _ = conn;
            mcp_tools::tool_repo_map(engine, max_tokens)
        }
        "hermes_check_consistency" => mcp_tools_consistency::tool_check_consistency_with_conn(engine, conn),
        "hermes_impact_analysis" => {
            let symbol = args["symbol_name"].as_str().unwrap_or("");
            anyhow::ensure!(!symbol.is_empty(), "hermes_impact_analysis requires 'symbol_name'");
            let _ = conn;
            mcp_tools::tool_impact_analysis(engine, symbol)
        }
        "hermes_mcp_status" => {
            let val = crate::mcp_status::tool_mcp_status(engine, project_root)?;
            Ok(serde_json::to_string_pretty(&val)?)
        }
        "hermes_tools" => {
            let intent = args["intent"].as_str().unwrap_or("all");
            let val = crate::tool_router::tools_payload_for_intent(intent);
            Ok(serde_json::to_string_pretty(&val)?)
        }
        "hermes_match_skills" => {
            let query = args["query"].as_str().unwrap_or("");
            anyhow::ensure!(!query.is_empty(), "hermes_match_skills requires 'query'");
            let scope = args["scope"].as_str();
            let val = crate::mcp_skills::tool_match_skills_with_conn(engine, conn, query, scope)?;
            Ok(serde_json::to_string_pretty(&val)?)
        }
        "hermes_fetch_skill" => {
            let skill_path = args["skill_path"].as_str().unwrap_or("");
            anyhow::ensure!(!skill_path.is_empty(), "hermes_fetch_skill requires 'skill_path'");
            let val = crate::mcp_skills::tool_fetch_skill_with_conn(engine, conn, skill_path)?;
            Ok(serde_json::to_string_pretty(&val)?)
        }
        "hermes_log_incident" => {
            anyhow::ensure!(
                args["title"].as_str().is_some_and(|t| !t.is_empty()),
                "hermes_log_incident requires 'title'"
            );
            crate::mcp_incidents::tool_log_incident(engine, project_root, args)
        }
        "hermes_resolve_incident" => {
            anyhow::ensure!(
                args["sub_product"].as_str().is_some() && args["slug"].as_str().is_some(),
                "hermes_resolve_incident requires 'sub_product' and 'slug'"
            );
            crate::mcp_incidents::tool_resolve_incident(engine, project_root, args)
        }
        "hermes_query_incidents" => crate::mcp_incidents::tool_query_incidents_with_conn(engine, conn, project_root, args),
        "hermes_list_tracks" => crate::mcp_tracks::tool_list_tracks_with_conn(engine, conn, project_root, args),
        "hermes_resume_track" => crate::mcp_tracks::tool_resume_track_with_conn(engine, conn, project_root, args),
        "hermes_write_kb_article" => {
            anyhow::ensure!(
                args["title"].as_str().is_some_and(|t| !t.is_empty()),
                "hermes_write_kb_article requires 'title'"
            );
            crate::mcp_incidents::tool_write_kb_article(engine, project_root, args)
        }
        "hermes_search_kb" => {
            anyhow::ensure!(args["query"].as_str().is_some_and(|q| !q.is_empty()), "hermes_search_kb requires 'query'");
            crate::mcp_incidents::tool_search_kb_with_conn(engine, conn, args)
        }
        "hermes_lint_architecture" => crate::mcp_lint::tool_lint_architecture(engine, project_root, args),
        "hermes_heal_violations" => crate::mcp_heal::tool_heal_violations(engine, project_root, args),
        "hermes_constraints" => {
            let file_path = args["file_path"].as_str().unwrap_or("");
            anyhow::ensure!(!file_path.is_empty(), "hermes_constraints requires 'file_path'");
            let _ = conn;
            crate::mcp_constraints::tool_constraints(engine, project_root, args)
        }
        "hermes_test_coverage_map" => crate::mcp_coverage::tool_test_coverage_map_with_conn(engine, conn, project_root, args),
        "hermes_search_misses" => {
            let since_days = args["since_days"].as_u64();
            let top_k = args["top_k"].as_u64().unwrap_or(10) as usize;
            let _ = conn;
            mcp_tools::tool_search_misses(engine, since_days, top_k)
        }
        "hermes_quality_review" => crate::mcp_quality::tool_quality_review(engine, project_root, args),
        "hermes_quality_score" => crate::mcp_quality::tool_quality_score(engine, project_root, args),
        "hermes_quality_next" => crate::mcp_quality::tool_quality_next(engine, project_root, args),
        "hermes_quality_resolve" => crate::mcp_quality::tool_quality_resolve(engine, project_root, args),
        "hermes_quality_wontfix" => crate::mcp_quality::tool_quality_wontfix(engine, project_root, args),
        "hermes_quality_dismiss" => crate::mcp_dismiss::tool_quality_dismiss(engine, project_root, args),
        "hermes_lint_dismiss" => crate::mcp_dismiss::tool_lint_dismiss(engine, project_root, args),
        "hermes_dismissed_list" => crate::mcp_dismiss::tool_dismissed_list(engine, project_root, args),
        "hermes_auto_dismiss" => crate::mcp_dismiss::tool_auto_dismiss(engine, project_root, args),
        "hermes_query_memory" => {
            let query = args["query"].as_str().unwrap_or("");
            anyhow::ensure!(!query.is_empty(), "hermes_query_memory requires 'query'");
            let limit = args["limit"]
                .as_u64()
                .map(|n| n as usize)
                .unwrap_or(crate::memory_query::DEFAULT_QUERY_LIMIT);
            crate::memory_query::tool_query_memory_with_conn(conn, query, limit)
        }
        "hermes_get_core_facts" => crate::memory_query::tool_get_core_facts_with_conn(conn, project_root),
        other => anyhow::bail!("unknown tool: {other}"),
    }
}
