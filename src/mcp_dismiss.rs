use anyhow::Result;
use chrono::{Duration, Utc};
use serde_json::{json, Value};
use std::path::Path;

use crate::quality::state::FindingStatus;
use crate::HermesEngine;

/// Dismiss a quality finding — it disappears from active lists but stays in DB.
pub fn tool_quality_dismiss(
    _engine: &HermesEngine,
    project_root: &Path,
    args: &Value,
) -> Result<String> {
    let id = args["id"].as_str().unwrap_or("");
    let reason = args["reason"].as_str().map(|s| s.to_string());
    anyhow::ensure!(!id.is_empty(), "id is required");

    let mut state = crate::quality::state::load(project_root)?;
    let found = apply_to_finding(&mut state, id, |f| {
        f.status = FindingStatus::Dismissed;
    })?;
    anyhow::ensure!(found, "finding {id} not found");

    recompute_all_scores(&mut state);
    crate::quality::state::save(project_root, &state)?;

    let mut result = json!({"dismissed": id, "project_score": state.project_score});
    if let Some(r) = reason {
        result["reason"] = json!(r);
    }
    Ok(result.to_string())
}

/// Dismiss a lint violation by fingerprint — stored in DB for cross-session filtering.
pub fn tool_lint_dismiss(
    engine: &HermesEngine,
    _project_root: &Path,
    args: &Value,
) -> Result<String> {
    let item_type = args["item_type"].as_str().unwrap_or("violation");
    let item_id = args["item_id"].as_str().unwrap_or("");
    let reason = args["reason"].as_str().map(|s| s.to_string());

    anyhow::ensure!(!item_id.is_empty(), "item_id is required");
    anyhow::ensure!(
        matches!(item_type, "violation" | "skill_candidate"),
        "item_type must be 'violation' or 'skill_candidate'"
    );

    let project_id = engine.project_id();
    let now = Utc::now().to_rfc3339();
    let _dismissed_by = reason.as_deref().map_or("system", |_| "user");

    let conn = engine.db();
    let db = conn.lock().unwrap();
    db.execute(
        "INSERT OR IGNORE INTO dismissed_items (project_id, item_type, item_id, reason, dismissed_by, dismissed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6);",
        rusqlite::params![
            project_id,
            item_type,
            item_id,
            reason.unwrap_or_default(),
            "user",
            now,
        ],
    )?;

    Ok(json!({
        "dismissed": true,
        "item_type": item_type,
        "item_id": item_id,
    }).to_string())
}

/// List all dismissed items for a project.
pub fn tool_dismissed_list(
    engine: &HermesEngine,
    _project_root: &Path,
    args: &Value,
) -> Result<String> {
    let item_type = args["item_type"].as_str();
    let limit = args["limit"].as_u64().unwrap_or(100) as usize;

    let project_id = engine.project_id();
    let conn = engine.db();
    let db = conn.lock().unwrap();

    let query = format!(
        "SELECT item_type, item_id, reason, dismissed_by, dismissed_at
         FROM dismissed_items
         WHERE project_id = ?1 {}
         ORDER BY dismissed_at DESC
         LIMIT ?2",
        if item_type.is_some() { "AND item_type = ?3" } else { "" }
    );

    let mut stmt;
    let rows: Vec<DismissedRow> = if let Some(it) = item_type {
        stmt = db.prepare(&query)?;
        stmt.query_map(rusqlite::params![project_id, limit as i64, it], |row| {
            Ok(DismissedRow {
                item_type: row.get(0)?,
                item_id: row.get(1)?,
                reason: row.get(2)?,
                dismissed_by: row.get(3)?,
                dismissed_at: row.get(4)?,
            })
        })?
        .filter_map(|r| r.ok())
        .take(limit)
        .collect()
    } else {
        stmt = db.prepare(&query)?;
        stmt.query_map(rusqlite::params![project_id, limit as i64], |row| {
            Ok(DismissedRow {
                item_type: row.get(0)?,
                item_id: row.get(1)?,
                reason: row.get(2)?,
                dismissed_by: row.get(3)?,
                dismissed_at: row.get(4)?,
            })
        })?
        .filter_map(|r| r.ok())
        .take(limit)
        .collect()
    };

    let items: Vec<Value> = rows.iter().map(|r| {
        let mut item = json!({
            "item_type": r.item_type,
            "item_id": r.item_id,
            "dismissed_by": r.dismissed_by,
            "dismissed_at": r.dismissed_at,
        });
        if !r.reason.is_empty() {
            item["reason"] = json!(r.reason);
        }
        item
    }).collect();

    Ok(json!({
        "dismissed_items": items,
        "count": items.len(),
    }).to_string())
}

/// Auto-dismiss open quality findings older than 30 days.
pub fn tool_auto_dismiss(
    _engine: &HermesEngine,
    project_root: &Path,
    args: &Value,
) -> Result<String> {
    let max_age_days = args["max_age_days"].as_u64().unwrap_or(30);

    let mut state = crate::quality::state::load(project_root)?;
    let cutoff = Utc::now() - Duration::days(max_age_days as i64);
    let mut auto_dismissed: Vec<String> = Vec::new();

    for ms in state.modules.values_mut() {
        for finding in &mut ms.findings {
            if !matches!(finding.status, FindingStatus::Open) {
                continue;
            }
            // Parse the detected timestamp and check age
            let detected = chrono::DateTime::parse_from_rfc3339(&finding.detected_ts);
            match detected {
                Ok(dt) if dt < cutoff => {
                    finding.status = FindingStatus::Dismissed;
                    auto_dismissed.push(finding.id.clone());
                }
                Err(_) => continue, // Skip entries with unparseable timestamps
                _ => {}              // Still in window, keep open
            }
        }
    }

    if !auto_dismissed.is_empty() {
        recompute_all_scores(&mut state);
        crate::quality::state::save(project_root, &state)?;
    }

    Ok(json!({
        "auto_dismissed": auto_dismissed.len(),
        "ids": auto_dismissed,
        "max_age_days": max_age_days,
    }).to_string())
}

/// Check whether an item has been dismissed (used by list queries to filter).
pub fn is_dismissed(engine: &HermesEngine, item_type: &str, item_id: &str) -> Result<bool> {
    let project_id = engine.project_id();
    let conn = engine.db();
    let db = conn.lock().unwrap();

    let count: i64 = db.query_row(
        "SELECT COUNT(*) FROM dismissed_items WHERE project_id = ?1 AND item_type = ?2 AND item_id = ?3",
        [project_id, item_type, item_id],
        |row| row.get(0),
    )?;

    Ok(count > 0)
}

/// Check if a quality finding is dismissed (reads from quality-state.json).
pub fn is_finding_dismissed(project_root: &Path, id: &str) -> Result<bool> {
    let state = crate::quality::state::load(project_root)?;
    for ms in state.modules.values() {
        if let Some(f) = ms.findings.iter().find(|ff| ff.id == id) {
            return Ok(matches!(f.status, FindingStatus::Dismissed));
        }
    }
    // If not found at all, treat as not dismissed (will fail with "not found" upstream)
    Ok(false)
}

fn apply_to_finding(state: &mut crate::quality::state::QualityState, id: &str, f: impl Fn(&mut crate::quality::state::Finding)) -> Result<bool> {
    for ms in state.modules.values_mut() {
        if let Some(finding) = ms.findings.iter_mut().find(|ff| ff.id == id) {
            f(finding);
            return Ok(true);
        }
    }
    Ok(false)
}

fn recompute_all_scores(state: &mut crate::quality::state::QualityState) {
    for ms in state.modules.values_mut() {
        ms.score_prev = ms.score;
        ms.score = crate::quality::score::compute_module_score(&ms.findings);
    }
    state.project_score_prev = state.project_score;
    state.project_score = crate::quality::score::compute_project_score(&state.modules);
}

struct DismissedRow {
    item_type: String,
    item_id: String,
    reason: String,
    dismissed_by: String,
    dismissed_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_dismissed_new_item_returns_false() {
        let engine = HermesEngine::in_memory("test-project").unwrap();
        assert!(!is_dismissed(&engine, "violation", "nonexistent-123").unwrap());
    }
}
