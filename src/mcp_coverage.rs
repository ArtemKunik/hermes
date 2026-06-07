// tools/hermes-engine/src/mcp_coverage.rs
// TRACK-045: MCP tool handler for hermes_test_coverage_map.

use anyhow::Result;
use rusqlite::params;
use serde_json::Value;
use std::path::Path;

use crate::graph::KnowledgeGraph;
use crate::ingestion::test_edge_builder::build_test_edges;
use crate::HermesEngine;

/// MCP tool: hermes_test_coverage_map
///
/// Parameters:
///   symbol (string?) — specific symbol name (default: all)
///   scope  (string?) — file or directory scope filter
pub fn tool_test_coverage_map(
    engine: &HermesEngine,
    project_root: &Path,
    args: &Value,
) -> Result<String> {
    let _symbol_filter = args["symbol"].as_str();
    let _scope_filter = args["scope"].as_str();

    let db = engine.read_db().lock().unwrap_or_else(|e| e.into_inner());
    tool_test_coverage_map_with_conn(engine, &db, project_root, args)
}

pub fn tool_test_coverage_map_with_conn(
    engine: &HermesEngine,
    conn: &rusqlite::Connection,
    project_root: &Path,
    args: &Value,
) -> Result<String> {
    let symbol_filter = args["symbol"].as_str();
    let scope_filter = args["scope"].as_str();

    let graph = KnowledgeGraph::from_conn(conn, engine.project_id());

    // Ensure Tests edges are populated (idempotent — existing edges are ignored via INSERT OR IGNORE)
    // build_test_edges needs to potentially write, so it'll use graph.with_conn
    let edges_added = build_test_edges(&graph, project_root)?;
    if edges_added > 0 {
        eprintln!("[hermes-coverage] built {edges_added} test→impl edges");
    }

    // Query: all implementation nodes that have at least one Tests edge (covered)
    let covered = query_covered(&graph, symbol_filter, scope_filter)?;
    // Query: all non-test function/struct nodes that have NO Tests edge (uncovered)
    let uncovered = query_uncovered(&graph, symbol_filter, scope_filter)?;

    let total = covered.len() + uncovered.len();
    let coverage_ratio = if total > 0 {
        covered.len() as f64 / total as f64
    } else {
        0.0
    };

    let output = serde_json::json!({
        "coverage": covered,
        "uncovered": uncovered,
        "coverage_ratio": (coverage_ratio * 1000.0).round() / 1000.0,
        "summary": {
            "covered": covered.len(),
            "uncovered": uncovered.len(),
            "total": total,
        }
    });
    Ok(serde_json::to_string_pretty(&output)?)
}

fn query_covered(
    graph: &KnowledgeGraph,
    symbol: Option<&str>,
    scope: Option<&str>,
) -> Result<Vec<Value>> {
    graph.with_conn(|conn: &rusqlite::Connection| {
        let mut stmt = conn.prepare(
            "SELECT n.name, n.file_path, n.node_type,
                    COUNT(e.source_id) AS test_count,
                    GROUP_CONCAT(ts.name || '|' || ts.file_path, ';;') AS test_info
             FROM nodes n
             JOIN edges e ON e.target_id = n.id AND e.edge_type = 'tests'
             JOIN nodes ts ON ts.id = e.source_id
             WHERE n.project_id = ?1
               AND n.node_type IN ('function', 'struct', 'impl')
               AND n.file_path NOT LIKE '%_test.rs'
               AND n.file_path NOT LIKE '%.test.%'
             GROUP BY n.id",
        )?;
        let rows = stmt.query_map(params![graph.project_id()], |row: &rusqlite::Row| {
            Ok((
                row.get::<_, String>(0).unwrap_or_default(), // name
                row.get::<_, String>(1).unwrap_or_default(), // file_path
                row.get::<_, String>(2).unwrap_or_default(), // node_type
                row.get::<_, i64>(3).unwrap_or(0),           // test_count
                row.get::<_, Option<String>>(4).unwrap_or_default().unwrap_or_default(), // test_info
            ))
        })?;

        let mut result = Vec::new();
        for row in rows.filter_map(|r| r.ok()) {
            let (name, fp, _nt, test_count, test_info): (String, String, String, i64, String) = row;
            if let Some(sym) = symbol {
                if !name.contains(sym) { continue; }
            }
            if let Some(scope_str) = scope {
                if !fp.replace('\\', "/").contains(scope_str) { continue; }
            }

            let tests: Vec<Value> = test_info
                .split(";;")
                .filter(|s: &&str| !s.is_empty())
                .map(|s: &str| {
                    let parts: Vec<&str> = s.split('|').collect();
                    serde_json::json!({ "name": parts.get(0).unwrap_or(&""), "path": parts.get(1).unwrap_or(&"") })
                })
                .collect();

            result.push(serde_json::json!({
                "symbol": name,
                "path": fp,
                "test_count": test_count,
                "tests": tests,
            }));
        }
        Ok(result)
    })
}

fn query_uncovered(
    graph: &KnowledgeGraph,
    symbol: Option<&str>,
    scope: Option<&str>,
) -> Result<Vec<Value>> {
    graph.with_conn(|conn: &rusqlite::Connection| {
        let mut stmt = conn.prepare(
            "SELECT n.name, n.file_path, n.node_type
             FROM nodes n
             LEFT JOIN edges e ON e.target_id = n.id AND e.edge_type = 'tests'
             WHERE n.project_id = ?1
               AND n.node_type IN ('function', 'struct', 'impl')
               AND n.file_path NOT LIKE '%_test.rs'
               AND n.file_path NOT LIKE '%.test.%'
               AND e.source_id IS NULL",
        )?;
        let rows = stmt.query_map(params![graph.project_id()], |row: &rusqlite::Row| {
            Ok((
                row.get::<_, String>(0).unwrap_or_default(),
                row.get::<_, String>(1).unwrap_or_default(),
                row.get::<_, String>(2).unwrap_or_default(),
            ))
        })?;

        let mut result = Vec::new();
        for row in rows.filter_map(|r| r.ok()) {
            let (name, fp, _nt): (String, String, String) = row;
            if let Some(sym) = symbol {
                if !name.contains(sym) {
                    continue;
                }
            }
            if let Some(scope_str) = scope {
                if !fp.replace('\\', "/").contains(scope_str) {
                    continue;
                }
            }
            result.push(serde_json::json!({
                "symbol": name,
                "path": fp,
            }));
        }
        Ok(result)
    })
}
