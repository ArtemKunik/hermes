use crate::graph::{KnowledgeGraph, Node, NodeType};
use anyhow::Result;
use rusqlite::params;
use std::collections::HashSet;

impl KnowledgeGraph {
    pub fn literal_search_by_name(&self, query: &str) -> Result<Vec<Node>> {
        let conn = self.db().lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let query_lower = query.to_lowercase();

        let prefix_pattern = format!("{}%", query_lower);
        let mut stmt = conn.prepare(
            "SELECT id, project_id, name, node_type, file_path, start_line, end_line, summary, content_hash
             FROM nodes WHERE project_id = ?1 AND LOWER(name) LIKE ?2",
        )?;
        let prefix_results: Vec<Node> = stmt
            .query_map(params![self.project_id(), prefix_pattern], node_from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        if !prefix_results.is_empty() {
            return Ok(prefix_results);
        }

        let contains_pattern = format!("%{}%", query_lower);
        let mut stmt2 = conn.prepare(
            "SELECT id, project_id, name, node_type, file_path, start_line, end_line, summary, content_hash
             FROM nodes WHERE project_id = ?1 AND LOWER(name) LIKE ?2",
        )?;
        let results: Vec<Node> = stmt2
            .query_map(params![self.project_id(), contains_pattern], node_from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(results)
    }

    pub fn get_all_file_paths(&self) -> Result<HashSet<String>> {
        let conn = self.db().lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut stmt = conn.prepare(
            "SELECT DISTINCT file_path FROM nodes
             WHERE project_id = ?1 AND node_type = 'file' AND file_path IS NOT NULL",
        )?;
        let paths = stmt
            .query_map(params![self.project_id()], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<HashSet<_>, _>>()?;
        Ok(paths)
    }

    pub fn delete_nodes_for_file(&self, file_path: &str) -> Result<()> {
        let conn = self.db().lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        conn.execute(
            "DELETE FROM fts_content WHERE node_id IN
             (SELECT id FROM nodes WHERE file_path = ?1 AND project_id = ?2)",
            params![file_path, self.project_id()],
        )?;
        conn.execute(
            "DELETE FROM edges WHERE
             source_id IN (SELECT id FROM nodes WHERE file_path = ?1 AND project_id = ?2)
             OR target_id IN (SELECT id FROM nodes WHERE file_path = ?1 AND project_id = ?2)",
            params![file_path, self.project_id()],
        )?;
        conn.execute(
            "DELETE FROM nodes WHERE file_path = ?1 AND project_id = ?2",
            params![file_path, self.project_id()],
        )?;
        Ok(())
    }

    pub fn get_all_nodes(&self) -> Result<Vec<Node>> {
        let conn = self.db().lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut stmt = conn.prepare(
            "SELECT id, project_id, name, node_type, file_path, start_line, end_line, summary, content_hash
             FROM nodes WHERE project_id = ?1",
        )?;
        let rows = stmt
            .query_map(params![self.project_id()], node_from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn fts_search(&self, query: &str, limit: usize) -> Result<Vec<(Node, f64)>> {
        let conn = self.db().lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut stmt = conn.prepare(
            "SELECT n.id, n.project_id, n.name, n.node_type, n.file_path, n.start_line, n.end_line, n.summary, n.content_hash,
                    bm25(fts_content) as rank
             FROM fts_content f
             JOIN nodes n ON n.id = f.node_id
             WHERE fts_content MATCH ?1 AND f.project_id = ?2
             ORDER BY rank
             LIMIT ?3",
        )?;
        let rows = stmt
            .query_map(params![query, self.project_id(), limit as i64], |row| {
                Ok((node_from_row(row)?, row.get::<_, f64>(9)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}

pub(crate) fn node_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Node> {
    Ok(Node {
        id: row.get(0)?,
        project_id: row.get(1)?,
        name: row.get(2)?,
        node_type: NodeType::parse_str(&row.get::<_, String>(3)?),
        file_path: row.get(4)?,
        start_line: row.get(5)?,
        end_line: row.get(6)?,
        summary: row.get(7)?,
        content_hash: row.get(8)?,
    })
}
