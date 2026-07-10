// tools/hermes-engine/src/mcp_tools_graph.rs
//
// Graph-traversal tools: repo_map and impact_analysis.
// Extracted from mcp_tools.rs for size compliance.

use anyhow::Result;
use rusqlite::Connection as SqliteConnection;
use serde_json::json;

use crate::{accounting::Accountant, blast_radius, graph::{Edge, EdgeType, KnowledgeGraph, Node, NodeType}, HermesEngine};

/// Generate a token-budget-constrained repository map.
///
/// Returns a compact listing of all code symbols (functions, structs, traits,
/// enums, interfaces) ranked by incoming edge count (xref frequency).
/// Packs symbols into the output until `max_tokens` is reached.
pub fn tool_repo_map(engine: &HermesEngine, max_tokens: usize) -> Result<String> {
    let conn = engine
        .read_db()
        .lock()
        .map_err(|e| anyhow::anyhow!("{e}"))?;
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
    let acct = Accountant::new(
        engine.db().clone(),
        engine.project_id(),
        engine.session_id(),
    );
    let ptr_tokens = token_count as u64;
    let traditional = (total_symbols as u64).saturating_mul(60).max(ptr_tokens);
    let _ = acct.record_query(
        &format!("repo_map:{max_tokens}"),
        ptr_tokens,
        0,
        traditional,
    );

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
    let conn = engine
        .read_db()
        .lock()
        .map_err(|e| anyhow::anyhow!("{e}"))?;
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
    let blast_info = get_blast_info(&conn, project_id, symbol_name);
    drop(stmt);
    drop(conn);
    let acct = Accountant::new(
        engine.db().clone(),
        engine.project_id(),
        engine.session_id(),
    );
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
        "affected_dependencies": impact_graph,
        "blast_radius": blast_info
    }))?)
}

/// Look up the blast-radius score for a specific node or file path.
pub fn tool_blast_score(engine: &HermesEngine, query: &str) -> Result<String> {
    let conn = engine
        .read_db()
        .lock()
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let project_id = engine.project_id();

    match crate::blast_radius::get_blast_score(&conn, project_id, query) {
        Ok(Some(score)) => {
            let affected = score.direct_count + score.transitive_count;
            Ok(serde_json::to_string_pretty(&json!({
                "node_id": score.node_id,
                "file_path": score.file_path,
                "direct_dependents": score.direct_count,
                "transitive_dependents": score.transitive_count,
                "total_affected": affected,
                "blast_score": score.blast_score,
                "risk_level": score.risk_level.as_str()
            }))?)
        }
        Ok(None) => anyhow::bail!("No blast-score data found for: {query}"),
        Err(e) => Err(e),
    }
}

/// Get top-N files/symbols by blast score above a threshold.
pub fn tool_high_blast(engine: &HermesEngine, threshold: f64, limit: usize) -> Result<String> {
    let conn = engine
        .read_db()
        .lock()
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let project_id = engine.project_id();

    let scores = crate::blast_radius::get_high_blast(&conn, project_id, threshold, limit)?;

    let items: Vec<serde_json::Value> = scores
        .iter()
        .map(|s| {
            json!({
                "node_id": s.node_id,
                "file_path": s.file_path,
                "direct_dependents": s.direct_count,
                "transitive_dependents": s.transitive_count,
                "blast_score": s.blast_score,
                "risk_level": s.risk_level.as_str()
            })
        })
        .collect();

    Ok(serde_json::to_string_pretty(&json!({
        "threshold": threshold,
        "limit": limit,
        "count": items.len(),
        "high_blast_nodes": items
    }))?)
}

/// Helper: look up blast info for a symbol name in impact_analysis.
fn get_blast_info(
    conn: &SqliteConnection,
    project_id: &str,
    symbol_name: &str,
) -> serde_json::Value {
    match blast_radius::get_blast_score(conn, project_id, symbol_name) {
        Ok(Some(score)) => json!({
            "found": true,
            "direct_dependents": score.direct_count,
            "transitive_dependents": score.transitive_count,
            "blast_score": score.blast_score,
            "risk_level": score.risk_level.as_str()
        }),
        _ => json!({ "found": false }),
    }
}

/// Get 1-hop neighbors of a node (both incoming and outgoing edges).
pub fn tool_neighbors(
    engine: &HermesEngine,
    node_id: &str,
    edge_types: Option<Vec<String>>,
    limit: Option<usize>,
) -> Result<String> {
    let conn = engine
        .read_db()
        .lock()
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let graph = KnowledgeGraph::new(conn.clone(), engine.project_id());

    let neighbors = graph.get_neighbors(node_id, edge_types.as_deref(), limit)?;

    let results: Vec<serde_json::Value> = neighbors
        .into_iter()
        .map(|(node, edge, is_outgoing)| {
            json!({
                "node": {
                    "id": node.id,
                    "project_id": node.project_id,
                    "name": node.name,
                    "node_type": node.node_type.as_str(),
                    "file_path": node.file_path,
                    "start_line": node.start_line,
                    "end_line": node.end_line,
                    "summary": node.summary,
                    "content_tokens": node.content_tokens,
                    "object_type": node.object_type,
                },
                "edge": {
                    "id": edge.id,
                    "project_id": edge.project_id,
                    "source_id": edge.source_id,
                    "target_id": edge.target_id,
                    "edge_type": edge.edge_type.as_str(),
                    "weight": edge.weight,
                },
                "direction": if is_outgoing { "outgoing" } else { "incoming" },
            })
        })
        .collect();

    let acct = Accountant::new(
        engine.db().clone(),
        engine.project_id(),
        engine.session_id(),
    );
    let ptr_tokens = (results.len() as u64).saturating_mul(40);
    let _ = acct.record_query(
        &format!("neighbors:{}", node_id),
        ptr_tokens,
        0,
        ptr_tokens.saturating_mul(15),
    );

    Ok(serde_json::to_string_pretty(&json!({
        "node_id": node_id,
        "neighbors": results,
        "count": results.len(),
    }))?)
}

/// Get a subgraph of nodes and edges with optional filters.
pub fn tool_graph(
    engine: &HermesEngine,
    node_ids: Option<Vec<String>>,
    node_types: Option<Vec<String>>,
    edge_types: Option<Vec<String>>,
    limit: Option<usize>,
) -> Result<String> {
    let conn = engine
        .read_db()
        .lock()
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let graph = KnowledgeGraph::new(conn.clone(), engine.project_id());

    let (nodes, edges) = graph.get_subgraph(
        node_ids.as_deref(),
        node_types.as_deref(),
        edge_types.as_deref(),
        limit,
    )?;

    let node_results: Vec<serde_json::Value> = nodes
        .into_iter()
        .map(|node| {
            json!({
                "id": node.id,
                "project_id": node.project_id,
                "name": node.name,
                "node_type": node.node_type.as_str(),
                "file_path": node.file_path,
                "start_line": node.start_line,
                "end_line": node.end_line,
                "summary": node.summary,
                "content_tokens": node.content_tokens,
                "object_type": node.object_type,
            })
        })
        .collect();

    let edge_results: Vec<serde_json::Value> = edges
        .into_iter()
        .map(|edge| {
            json!({
                "id": edge.id,
                "project_id": edge.project_id,
                "source_id": edge.source_id,
                "target_id": edge.target_id,
                "edge_type": edge.edge_type.as_str(),
                "weight": edge.weight,
            })
        })
        .collect();

    let total_count = node_results.len() + edge_results.len();
    let acct = Accountant::new(
        engine.db().clone(),
        engine.project_id(),
        engine.session_id(),
    );
    let ptr_tokens = (total_count as u64).saturating_mul(30);
    let _ = acct.record_query(
        "graph",
        ptr_tokens,
        0,
        ptr_tokens.saturating_mul(15),
    );

    Ok(serde_json::to_string_pretty(&json!({
        "nodes": node_results,
        "edges": edge_results,
        "node_count": node_results.len(),
        "edge_count": edge_results.len(),
    }))?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        graph::KnowledgeGraph, graph::NodeType, graph_builders::NodeBuilder, HermesEngine,
    };

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
