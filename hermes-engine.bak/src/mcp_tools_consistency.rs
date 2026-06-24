// tools/hermes-engine/src/mcp_tools_consistency.rs
//
// Active Guardian: environment-variable consistency tool.
// Extracted from mcp_tools_validation.rs for 300-line file limit compliance.

use anyhow::Result;
use serde_json::json;
use std::collections::{HashMap, HashSet};

use crate::HermesEngine;

/// Active Guardian: Check consistency of environment variables.
///
/// Scans the knowledge graph for Config nodes and reports:
/// - "Unknown": Used in code but not defined in .env/docs.
/// - "Unused": Defined in .env/docs but not used in code.
pub fn tool_check_consistency(engine: &HermesEngine) -> Result<String> {
    let db = engine.read_db().lock().unwrap_or_else(|e| e.into_inner());
    tool_check_consistency_with_conn(engine, &db)
}

pub fn tool_check_consistency_with_conn(
    engine: &HermesEngine,
    conn: &rusqlite::Connection,
) -> Result<String> {
    let project_id = engine.project_id();

    let mut stmt = conn.prepare(
        "SELECT n.name, e.edge_type, src.file_path \
         FROM nodes n \
         JOIN edges e ON e.target_id = n.id \
         JOIN nodes src ON src.id = e.source_id \
         WHERE n.node_type = 'config' AND n.project_id = ?",
    )?;

    let rows = stmt.query_map([project_id], |row| {
        Ok((
            row.get::<_, String>(0)?, // name
            row.get::<_, String>(1)?, // edge_type
            row.get::<_, String>(2)?, // file_path
        ))
    })?;

    let mut configs: HashMap<String, (HashSet<String>, HashSet<String>)> = HashMap::new();
    for row in rows {
        let (name, edge_type, file_path) = row?;
        let entry = configs.entry(name).or_insert((HashSet::new(), HashSet::new()));
        if edge_type == "defines" {
            entry.0.insert(file_path);
        } else if edge_type == "uses" {
            entry.1.insert(file_path);
        }
    }

    let defined_names: Vec<String> = configs
        .iter()
        .filter(|(_, (defs, _))| !defs.is_empty())
        .map(|(name, _)| name.clone())
        .collect();

    let mut unknown = Vec::new();
    let mut unused = Vec::new();
    let mut consistent = Vec::new();

    for (name, (definitions, usages)) in configs {
        if definitions.is_empty() && !usages.is_empty() {
            let suggestion = defined_names
                .iter()
                .filter_map(|def| {
                    let dist = strsim::levenshtein(&name, def);
                    if dist <= 3 { Some((def.clone(), dist)) } else { None }
                })
                .min_by_key(|(_, d)| *d)
                .map(|(s, _)| s);
            unknown.push(json!({
                "variable": name,
                "used_in": usages.into_iter().collect::<Vec<_>>(),
                "suggestion": suggestion
            }));
        } else if !definitions.is_empty() && usages.is_empty() {
            unused.push(json!({
                "variable": name,
                "defined_in": definitions.into_iter().collect::<Vec<_>>()
            }));
        } else {
            consistent.push(json!({
                "variable": name,
                "defined_in": definitions.into_iter().collect::<Vec<_>>(),
                "used_in": usages.into_iter().collect::<Vec<_>>()
            }));
        }
    }

    Ok(serde_json::to_string_pretty(&json!({
        "status": if unknown.is_empty() && unused.is_empty() { "clear" } else { "issues_found" },
        "summary": {
            "unknown_count": unknown.len(),
            "unused_count": unused.len(),
            "consistent_count": consistent.len()
        },
        "unknown_variables": unknown,
        "unused_variables": unused,
        "consistent_variables": consistent
    }))?)
}
