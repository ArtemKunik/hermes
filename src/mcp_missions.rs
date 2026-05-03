// tools/hermes-engine/src/mcp_missions.rs
//
// MCP tool implementations for mission lifecycle.

use anyhow::Result;
use rusqlite::Connection;
use serde_json::{json, Value};

use crate::mission::MissionStore;
use crate::HermesEngine;

/// `hermes_mission_start` — create a new mission in `preflight` status.
pub fn tool_mission_start(
    engine: &HermesEngine,
    conn: &Connection,
    args: &Value,
) -> Result<String> {
    let title = args["title"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("mission_start requires 'title'"))?;
    let description = args["description"].as_str();
    let tags = args["tags"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(","));

    let store = MissionStore::new(conn, engine.project_id());
    let mission = store.create(title, description, tags.as_deref())?;
    Ok(serde_json::to_string_pretty(&json!({
        "mission_id": mission.id,
        "status": mission.status,
        "created_at": mission.created_at,
    }))?)
}

/// `hermes_mission_update` — transition status and/or update metadata.
pub fn tool_mission_update(
    engine: &HermesEngine,
    conn: &Connection,
    args: &Value,
) -> Result<String> {
    let mission_id = args["mission_id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("mission_update requires 'mission_id'"))?;

    let store = MissionStore::new(conn, engine.project_id());

    // Metadata updates (non-status fields)
    store.update_metadata(mission_id, args)?;

    // Status transition
    let mission = if let Some(new_status) = args["status"].as_str() {
        let m = store.update_status(mission_id, new_status)?;

        // Auto-recall on transition to active
        if new_status == "active" {
            let recall_ctx = auto_recall_for_mission(engine, conn, &m);
            if let Ok(ctx) = recall_ctx {
                store.append_log(mission_id, "auto_recall", &ctx)?;
            }
        }
        // Auto-review placeholder on transition to landing
        if new_status == "landing" {
            store.append_log(
                mission_id,
                "auto_review",
                &json!({"note": "no reviewer configured — skipped"}),
            )?;
        }
        m
    } else {
        store.get(mission_id)?
    };

    // Append arbitrary events from args
    if let Some(event_type) = args["event_type"].as_str() {
        let event_data = args.get("event_data").cloned().unwrap_or(json!({}));
        store.append_log(mission_id, event_type, &event_data)?;
    }

    let log = store.get_log(mission_id)?;
    Ok(serde_json::to_string_pretty(&json!({
        "mission": mission,
        "log_entries": log.len(),
    }))?)
}

/// `hermes_mission_list` — list missions with optional status filter.
pub fn tool_mission_list(
    engine: &HermesEngine,
    conn: &Connection,
    args: &Value,
) -> Result<String> {
    let status = args["status"].as_str();
    let limit = args["limit"].as_u64().unwrap_or(20) as usize;
    let store = MissionStore::new(conn, engine.project_id());
    let missions = store.list(status, limit)?;
    Ok(serde_json::to_string_pretty(&missions)?)
}

/// `hermes_mission_status` — fetch a mission and its full event log.
pub fn tool_mission_status(
    engine: &HermesEngine,
    conn: &Connection,
    args: &Value,
) -> Result<String> {
    let mission_id = args["mission_id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("mission_status requires 'mission_id'"))?;
    let store = MissionStore::new(conn, engine.project_id());
    let mission = store.get(mission_id)?;
    let log = store.get_log(mission_id)?;
    Ok(serde_json::to_string_pretty(&json!({
        "mission": mission,
        "log": log,
    }))?)
}

/// `hermes_mission_event` — append a timestamped event to mission log.
pub fn tool_mission_event(
    engine: &HermesEngine,
    conn: &Connection,
    args: &Value,
) -> Result<String> {
    let mission_id = args["mission_id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("mission_event requires 'mission_id'"))?;
    let event_type = args["event_type"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("mission_event requires 'event_type'"))?;
    let data = args.get("data").cloned().unwrap_or(json!({}));

    let store = MissionStore::new(conn, engine.project_id());
    store.append_log(mission_id, event_type, &data)?;
    let log = store.get_log(mission_id)?;
    Ok(serde_json::to_string_pretty(&json!({
        "mission_id": mission_id,
        "event_type": event_type,
        "log_entries": log.len(),
    }))?)
}

// ── Auto-recall helper ───────────────────────────────────────────────────

fn auto_recall_for_mission(
    engine: &HermesEngine,
    conn: &Connection,
    mission: &crate::mission::Mission,
) -> Result<Value> {
    let query = mission.title.as_str();
    let recall_result = crate::mcp_memory::tool_recall_with_conn(engine, conn, query)?;
    let parsed: Value = serde_json::from_str(&recall_result).unwrap_or(json!({}));
    Ok(json!({
        "source": "auto_recall",
        "query": query,
        "has_prior_work": parsed["has_prior_work"],
    }))
}
