// tools/hermes-engine/src/mcp_tools_stats.rs
//
// Stats and fact-recording tools. Extracted from mcp_tools.rs for size compliance.

use anyhow::Result;
use rusqlite::Connection;
use serde_json::json;

use crate::{accounting::Accountant, temporal::TemporalStore, HermesEngine};

/// Version used by the MCP actor: takes an already-locked connection to avoid
/// re-locking the shared read_db mutex (which would deadlock on in-memory engines).
pub fn tool_stats_with_conn(engine: &HermesEngine, conn: &Connection) -> Result<String> {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    let acct = Accountant::new_with_conn(conn, engine.project_id(), engine.session_id());
    let since_midnight = Duration::from_secs(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            % 86400,
    );
    let session = acct.get_stats_since(Some(since_midnight))?;
    let cumulative = acct.get_cumulative_stats()?;
    let by_tool = acct
        .get_stats_by_tool(None)?
        .into_iter()
        .map(|(tool, queries, saved)| {
            json!({"tool": tool, "queries": queries, "tokens_saved": saved})
        })
        .collect::<Vec<_>>();
    let impact = acct.get_impact_summary()?;
    Ok(serde_json::to_string_pretty(&json!({
        "session": {
            "total_queries":            session.total_queries,
            "pointer_tokens_used":      session.total_pointer_tokens,
            "fetched_tokens_used":      session.total_fetched_tokens,
            "traditional_rag_estimate": session.total_traditional_estimate,
            "tokens_saved":             session.cumulative_savings_tokens,
            "savings_pct":              format!("{:.1}%", session.cumulative_savings_pct),
        },
        "cumulative": {
            "total_queries":            cumulative.total_queries,
            "pointer_tokens_used":      cumulative.total_pointer_tokens,
            "fetched_tokens_used":      cumulative.total_fetched_tokens,
            "traditional_rag_estimate": cumulative.total_traditional_estimate,
            "tokens_saved":             cumulative.cumulative_savings_tokens,
            "savings_pct":              format!("{:.1}%", cumulative.cumulative_savings_pct),
        },
        "by_tool": by_tool,
        "impact": impact,
    }))?)
}

/// CLI / non-actor version: acquires the diagnostic DB connection internally.
pub fn tool_stats(engine: &HermesEngine) -> Result<String> {
    let diagnostic_db = engine.diagnostic_db()?;
    let conn = diagnostic_db
        .as_ref()
        .lock()
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    tool_stats_with_conn(engine, &conn)
}

pub fn tool_add_fact(engine: &HermesEngine, fact_type_str: &str, content: &str) -> Result<String> {
    let diagnostic_db = engine.diagnostic_db()?;
    let conn = diagnostic_db
        .as_ref()
        .lock()
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    tool_add_fact_with_conn(engine, &conn, fact_type_str, content)
}

pub fn tool_add_fact_with_conn(
    engine: &HermesEngine,
    conn: &Connection,
    fact_type_str: &str,
    content: &str,
) -> Result<String> {
    use crate::temporal::{AddFactInput, FactType};
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
    let store = TemporalStore::from_conn(conn, engine.project_id());
    let id = store.add_fact(input)?;
    Ok(serde_json::to_string_pretty(
        &json!({ "id": id, "status": "recorded" }),
    )?)
}

pub fn tool_list_facts(engine: &HermesEngine, filter: Option<&str>) -> Result<String> {
    use crate::temporal::{FactFilter, FactType};
    let store = TemporalStore::new(engine.read_db().clone(), engine.project_id());
    let fact_filter = FactFilter {
        fact_type: filter.map(FactType::parse_str),
        ..Default::default()
    };
    let facts = store.get_active_facts(&fact_filter)?;
    Ok(serde_json::to_string_pretty(&facts)?)
}
