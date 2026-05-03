// tools/hermes-engine/src/mcp_tools_graph.rs
//
// Graph-traversal tools: repo_map and impact_analysis.
// Extracted from mcp_tools.rs for size compliance.

use anyhow::Result;
use serde_json::json;

use crate::{accounting::Accountant, HermesEngine};

/// Generate a token-budget-constrained repository map.
///
/// Returns a compact listing of all code symbols (functions, structs, traits,
/// enums, interfaces) ranked by incoming edge count (xref frequency).
/// Packs symbols into the output until `max_tokens` is reached.
pub fn tool_repo_map(engine: &HermesEngine, max_tokens: usize) -> Result<String> {
    let conn = engine.read_db().lock().map_err(|e| anyhow::anyhow!("{e}"))?;
    let project_id = engine.project_id();

    let mut stmt = conn.prepare(
        "SELECT n.name, n.node_type, n.object_type, n.file_path, n.start_line, \
                COALESCE(e.ref_count, 0) as ref_count \
         FROM nodes n \
         LEFT JOIN ( \
             SELECT target_id, COUNT(*) as ref_count \
             FROM edges WHERE project_id = ?1 \
             GROUP BY target_id \
         ) e ON e.target_id = n.id \
         WHERE n.project_id = ?1 \
           AND n.node_type NOT IN ('file', 'concept', 'document') \
           AND n.name IS NOT NULL AND n.name != '' \
         ORDER BY COALESCE(e.ref_count, 0) DESC, n.name",
    )?;

    #[allow(clippy::type_complexity)]
    let rows: Vec<(
        String,
        String,
        Option<String>,
        Option<String>,
        Option<i64>,
        i64,
    )> = stmt
        .query_map([project_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    let mut lines = Vec::new();
    let mut token_count = 0usize;

    for (name, node_type, object_type, file_path, start_line, refs) in &rows {
        let kind = object_type.as_deref().unwrap_or(node_type.as_str());
        let loc = match (file_path.as_deref(), start_line) {
            (Some(fp), Some(ln)) => format!(" [{fp}:{ln}]"),
            (Some(fp), None) => format!(" [{fp}]"),
            _ => String::new(),
        };
        let line = format!("{kind} {name}{loc} refs:{refs}");
        let line_tokens = line.split_whitespace().count() * 4 / 3 + 2;
        if token_count + line_tokens > max_tokens {
            break;
        }
        token_count += line_tokens;
        lines.push(line);
    }

    let total_symbols = rows.len();
    drop(stmt);
    drop(conn);
    let acct = Accountant::new(engine.db().clone(), engine.project_id(), engine.session_id());
    let ptr_tokens = token_count as u64;
    let traditional = (total_symbols as u64).saturating_mul(60).max(ptr_tokens);
    let _ = acct.record_query(&format!("repo_map:{max_tokens}"), ptr_tokens, 0, traditional);

    Ok(serde_json::to_string_pretty(&json!({
        "total_symbols": total_symbols,
        "included": lines.len(),
        "token_estimate": token_count,
        "max_tokens": max_tokens,
        "map": lines.join("\n"),
    }))?)
}

/// Analyze the potential "blast radius" of changing a symbol.
///
/// Traces incoming edges (Calls, Imports, Uses) up the graph to find
/// all direct and indirect dependencies.
pub fn tool_impact_analysis(engine: &HermesEngine, symbol_name: &str) -> Result<String> {
    let conn = engine.read_db().lock().map_err(|e| anyhow::anyhow!("{e}"))?;
    let project_id = engine.project_id();

    // 1. Find the target node(s)
    let mut stmt = conn.prepare(
        "SELECT id, name, node_type, file_path FROM nodes \
         WHERE project_id = ? AND name = ? COLLATE NOCASE",
    )?;
    let targets: Vec<(String, String, String, Option<String>)> = stmt
        .query_map([project_id, symbol_name], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    if targets.is_empty() {
        anyhow::bail!("Symbol not found: {symbol_name}");
    }

    use std::collections::{HashSet, VecDeque};
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();
    let mut impact_graph = Vec::new();

    for (id, name, kind, file) in &targets {
        visited.insert(id.clone());
        queue.push_back((id.clone(), name.clone(), kind.clone(), file.clone(), 0));
    }

    // 2. BFS upwards (following incoming edges)
    while let Some((id, name, kind, file, depth)) = queue.pop_front() {
        if depth > 0 {
            impact_graph.push(json!({
                "symbol": name,
                "type": kind,
                "file": file,
                "depth": depth
            }));
        }

        if depth >= 3 {
            continue;
        } // Limit depth to avoid massive graphs

        let mut stmt = conn.prepare(
            "SELECT src.id, src.name, src.node_type, src.file_path, e.edge_type \
             FROM edges e \
             JOIN nodes src ON src.id = e.source_id \
             WHERE e.target_id = ? AND e.project_id = ?",
        )?;

        let upstream = stmt.query_map([id, project_id.to_string()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;

        for up in upstream {
            let (uid, uname, ukind, ufile, _etype) = up?;
            if !visited.contains(&uid) {
                visited.insert(uid.clone());
                queue.push_back((uid, uname, ukind, ufile, depth + 1));
            }
        }
    }

    let impact_len = impact_graph.len();
    let target_len = targets.len();
    drop(stmt);
    drop(conn);
    let acct = Accountant::new(engine.db().clone(), engine.project_id(), engine.session_id());
    let ptr_tokens = ((impact_len + target_len) as u64).saturating_mul(40);
    let _ = acct.record_query(
        &format!("impact:{symbol_name}"),
        ptr_tokens,
        0,
        ptr_tokens.saturating_mul(15),
    );

    Ok(serde_json::to_string_pretty(&json!({
        "symbol": symbol_name,
        "impact_score": impact_len,
        "affected_dependencies": impact_graph
    }))?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{graph::KnowledgeGraph, graph::NodeType, graph_builders::NodeBuilder, HermesEngine};

    #[test]
    fn test_repo_map_empty_graph() {
        let engine = HermesEngine::in_memory("test-map").unwrap();
        let result: serde_json::Value =
            serde_json::from_str(&tool_repo_map(&engine, 2048).unwrap()).unwrap();
        assert_eq!(result["total_symbols"], 0);
        assert_eq!(result["included"], 0);
        assert_eq!(result["map"], "");
    }

    #[test]
    fn test_repo_map_respects_token_budget() {
        let engine = HermesEngine::in_memory("test-map-budget").unwrap();
        let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
        // Add 50 function nodes — should exceed a tiny budget
        for i in 0..50 {
            let node = NodeBuilder::new("test-map-budget")
                .name(&format!("function_{i}"))
                .node_type(NodeType::Function)
                .file_path("src/lib.rs")
                .build();
            graph.add_node(&node).unwrap();
        }
        // Give a budget of ~30 tokens — should include only a few symbols
        let result: serde_json::Value =
            serde_json::from_str(&tool_repo_map(&engine, 30).unwrap()).unwrap();
        assert_eq!(result["total_symbols"], 50);
        let included = result["included"].as_u64().unwrap();
        assert!(
            included > 0 && included < 50,
            "included={included}, expected partial"
        );
    }
}
