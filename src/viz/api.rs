use crate::HermesEngine;
use anyhow::Result;
use rusqlite::params;
use serde_json::{json, Value};

pub fn get_graph_json(engine: &HermesEngine) -> Result<Value> {
    let conn = engine
        .read_db()
        .lock()
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    // File nodes with blast scores
    let mut node_stmt = conn.prepare(
        "SELECT DISTINCT n.id, n.name, n.file_path, n.object_type, n.content_tokens,
                COALESCE(bs.blast_score, 0.0) as score,
                COALESCE(bs.risk_level, 'LOW') as risk,
                n.start_line, n.end_line
         FROM nodes n
         LEFT JOIN blast_scores bs ON bs.node_id = n.id AND bs.project_id = ?1
         WHERE n.project_id = ?1 AND n.file_path IS NOT NULL
         ORDER BY score DESC
         LIMIT 200",
    )?;

    let mut nodes = Vec::new();
    let node_rows = node_stmt.query_map([engine.project_id()], |row| {
        let file_path: Option<String> = row.get(2)?;
        let loc: i64 = row
            .get(7)
            .unwrap_or(0)
            .max(row.get(8).unwrap_or(0) - row.get(7).unwrap_or(0) + 1);
        Ok(json!({
            "id": row.get::<_, String>(0)?,
            "name": row.get::<_, String>(1)?,
            "file": file_path.unwrap_or_default(),
            "object_type": row.get::<_, Option<String>>(3)?,
            "loc": loc,
            "blast_score": row.get::<_, f64>(5)?,
            "risk": row.get::<_, String>(6)?,
        }))
    })?;
    for r in node_rows {
        nodes.push(r?);
    }

    // Dependency edges
    let mut edge_stmt = conn.prepare(
        "SELECT e.source_id, e.target_id, e.edge_type
         FROM edges e
         WHERE e.project_id = ?1
           AND e.edge_type IN ('calls', 'imports', 'uses', 'depends_on', 'implements')
         LIMIT 500",
    )?;

    let mut edges = Vec::new();
    let edge_rows = edge_stmt.query_map([engine.project_id()], |row| {
        Ok(json!({
            "source": row.get::<_, String>(0)?,
            "target": row.get::<_, String>(1)?,
            "type": row.get::<_, String>(2)?,
        }))
    })?;
    for r in edge_rows {
        edges.push(r?);
    }

    Ok(json!({ "nodes": nodes, "edges": edges }))
}

pub fn get_blast_json(engine: &HermesEngine, threshold: f64) -> Result<Value> {
    let conn = engine
        .read_db()
        .lock()
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let mut stmt = conn.prepare(
        "SELECT bs.file_path, bs.blast_score, bs.risk_level,
                bs.direct_count, bs.transitive_count
         FROM blast_scores bs
         WHERE bs.project_id = ?1 AND bs.blast_score >= ?2
         ORDER BY bs.blast_score DESC
         LIMIT 200",
    )?;

    let mut results = Vec::new();
    let rows = stmt.query_map(params![engine.project_id(), threshold], |row| {
        Ok(json!({
            "file_path": row.get::<_, Option<String>>(0)?,
            "blast_score": row.get::<_, f64>(1)?,
            "risk_level": row.get::<_, String>(2)?,
            "direct_count": row.get::<_, i64>(3)?,
            "transitive_count": row.get::<_, i64>(4)?,
        }))
    })?;
    for r in rows {
        results.push(r?);
    }

    Ok(serde_json::Value::Array(results))
}

pub fn get_symbols_json(engine: &HermesEngine, file_path: &str) -> Result<Value> {
    let conn = engine
        .read_db()
        .lock()
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let mut stmt = conn.prepare(
        "SELECT name, line, kind, exported
         FROM symbol_index
         WHERE project_id = ?1 AND file_path = ?2
         ORDER BY line",
    )?;

    let mut results = Vec::new();
    let rows = stmt.query_map(params![engine.project_id(), file_path], |row| {
        Ok(json!({
            "name": row.get::<_, String>(0)?,
            "line": row.get::<_, i64>(1)?,
            "kind": row.get::<_, String>(2)?,
            "exported": row.get::<_, bool>(3)?,
        }))
    })?;
    for r in rows {
        results.push(r?);
    }

    Ok(serde_json::Value::Array(results))
}
