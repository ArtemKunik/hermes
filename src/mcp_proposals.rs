// tools/hermes-engine/src/mcp_proposals.rs
//
// MCP tool implementations for proposal lifecycle.

use anyhow::Result;
use rusqlite::Connection;
use serde_json::{json, Value};

use crate::proposal::ProposalStore;
use crate::HermesEngine;

pub fn tool_proposal_create(
    engine: &HermesEngine,
    conn: &Connection,
    args: &Value,
) -> Result<String> {
    let proposals = args["proposals"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("proposal_create requires 'proposals' array"))?;

    let store = ProposalStore::new(conn, engine.project_id());
    let created = store.create_batch(proposals)?;
    Ok(serde_json::to_string_pretty(&json!({
        "created": created.len(),
        "proposals": created,
    }))?)
}

pub fn tool_proposal_list(
    engine: &HermesEngine,
    conn: &Connection,
    args: &Value,
) -> Result<String> {
    let status = args["status"].as_str();
    let source = args["source"].as_str();
    let limit = args["limit"].as_u64().unwrap_or(50) as usize;

    let store = ProposalStore::new(conn, engine.project_id());
    let proposals = store.list(status, source, limit)?;
    Ok(serde_json::to_string_pretty(&proposals)?)
}

pub fn tool_proposal_update(
    engine: &HermesEngine,
    conn: &Connection,
    args: &Value,
) -> Result<String> {
    let proposal_id = args["proposal_id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("proposal_update requires 'proposal_id'"))?;

    let store = ProposalStore::new(conn, engine.project_id());
    let proposal = store.update(proposal_id, args)?;
    Ok(serde_json::to_string_pretty(&proposal)?)
}

pub fn tool_proposal_reject(
    engine: &HermesEngine,
    conn: &Connection,
    args: &Value,
) -> Result<String> {
    let proposal_id = args["proposal_id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("proposal_reject requires 'proposal_id'"))?;
    let reason = args["reason"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("proposal_reject requires 'reason'"))?;

    let store = ProposalStore::new(conn, engine.project_id());
    let proposal = store.reject(proposal_id, reason)?;
    Ok(serde_json::to_string_pretty(&proposal)?)
}

pub fn tool_proposal_approve(
    engine: &HermesEngine,
    conn: &Connection,
    args: &Value,
) -> Result<String> {
    let proposal_id = args["proposal_id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("proposal_approve requires 'proposal_id'"))?;

    let store = ProposalStore::new(conn, engine.project_id());
    let result = store.approve(engine, conn, proposal_id)?;
    Ok(serde_json::to_string_pretty(&result)?)
}
