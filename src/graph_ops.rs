use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{params, Connection};
use std::sync::{Arc, Mutex};

use super::graph_types::*;
use crate::graph_builders::{EdgeBuilder, NodeBuilder};
use crate::lock_ext::LockExt;
use crate::{neural_embed, vector_ops};

pub struct KnowledgeGraph {
    db: Arc<Mutex<Connection>>,
    project_id: String,
}

impl KnowledgeGraph {
    pub fn new(db: Arc<Mutex<Connection>>, project_id: &str) -> Self {
        Self {
            db,
            project_id: project_id.to_string(),
        }
    }

    pub fn add_node(&self, node: &Node) -> Result<()> {
        let conn = self.db.lock_ctx("add_node")?;
        let now = Utc::now().to_rfc3339();
        let blob = node_vector_blob(node);
        conn.execute(
            "INSERT OR REPLACE INTO nodes
             (id, project_id, name, node_type, file_path, start_line, end_line, summary, content_hash, updated_at, vector)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                node.id, node.project_id, node.name, node.node_type.as_str(),
                node.file_path, node.start_line, node.end_line, node.summary,
                node.content_hash, now, blob,
            ],
        )?;
        Ok(())
    }

    pub fn get_all_node_vectors(&self) -> Result<Vec<(Node, Vec<f32>)>> {
        let conn = self.db.lock_ctx("get_all_node_vectors")?;
        let mut stmt = conn.prepare(
            "SELECT id, project_id, name, node_type, file_path, start_line, end_line, summary, content_hash, vector
             FROM nodes WHERE project_id = ?1 AND vector IS NOT NULL",
        )?;
        let rows = stmt
            .query_map(params![self.project_id], |row| {
                let node = crate::graph_queries::node_from_row(row)?;
                let blob: Vec<u8> = row.get(9)?;
                Ok((node, blob))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows
            .into_iter()
            .map(|(node, blob)| (node, vector_ops::blob_to_vec(&blob)))
            .collect())
    }

    pub fn get_node_vectors_by_ids(&self, node_ids: &[String]) -> Result<Vec<(Node, Vec<f32>)>> {
        if node_ids.is_empty() {
            return Ok(Vec::new());
        }

        let conn = self.db.lock_ctx("get_node_vectors_by_ids")?;
        let placeholders = std::iter::repeat("?")
            .take(node_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT id, project_id, name, node_type, file_path, start_line, end_line, summary, content_hash, vector
             FROM nodes
             WHERE project_id = ?1 AND vector IS NOT NULL AND id IN ({placeholders})"
        );
        let mut stmt = conn.prepare(&sql)?;
        let params_iter =
            std::iter::once(self.project_id.as_str()).chain(node_ids.iter().map(String::as_str));
        let rows = stmt
            .query_map(rusqlite::params_from_iter(params_iter), |row| {
                let node = crate::graph_queries::node_from_row(row)?;
                let blob: Vec<u8> = row.get(9)?;
                Ok((node, blob))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows
            .into_iter()
            .map(|(node, blob)| (node, vector_ops::blob_to_vec(&blob)))
            .collect())
    }

    /// Write a file node, its FTS entry, all changed chunk nodes/edges, and
    /// the associated chunk hashes in a single SQLite transaction.  One lock
    /// acquisition, one BEGIN/COMMIT — much faster than N individual calls.
    pub fn ingest_file_batch(
        &self,
        file_node: &Node,
        file_content: &str,
        chunks: &[ChunkWriteRecord],
    ) -> Result<()> {
        let conn = self.db.lock_ctx("ingest_file_batch")?;
        conn.execute_batch("BEGIN")?;
        let result = Self::do_ingest_file_batch(&conn, file_node, file_content, chunks);
        if result.is_ok() {
            conn.execute_batch("COMMIT")?;
        } else {
            let _ = conn.execute_batch("ROLLBACK");
        }
        result
    }

    fn do_ingest_file_batch(
        conn: &Connection,
        file_node: &Node,
        file_content: &str,
        chunks: &[ChunkWriteRecord],
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();

        let file_vec_blob = node_vector_blob(file_node);
        conn.execute(
            "INSERT OR REPLACE INTO nodes
             (id, project_id, name, node_type, file_path, start_line, end_line, summary, content_hash, updated_at, vector)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                file_node.id, file_node.project_id, file_node.name,
                file_node.node_type.as_str(), file_node.file_path,
                file_node.start_line, file_node.end_line, file_node.summary,
                file_node.content_hash, now, file_vec_blob,
            ],
        )?;
        conn.execute(
            "DELETE FROM fts_content WHERE node_id = ?1",
            params![file_node.id],
        )?;
        conn.execute(
            "INSERT INTO fts_content (node_id, project_id, name, content, file_path)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                file_node.id,
                file_node.project_id,
                file_node.name,
                file_content,
                file_node.file_path,
            ],
        )?;
        if let (Some(file_path), Some(content_hash)) = (
            file_node.file_path.as_deref(),
            file_node.content_hash.as_deref(),
        ) {
            conn.execute(
                "INSERT OR REPLACE INTO file_hashes (file_path, project_id, content_hash, indexed_at)
                 VALUES (?1, ?2, ?3, datetime('now'))",
                params![file_path, file_node.project_id, content_hash],
            )?;
        }

        for record in chunks {
            let chunk_vec_blob = node_vector_blob(&record.node);
            conn.execute(
                "INSERT OR REPLACE INTO nodes
                 (id, project_id, name, node_type, file_path, start_line, end_line, summary, content_hash, updated_at, vector)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    record.node.id, record.node.project_id, record.node.name,
                    record.node.node_type.as_str(), record.node.file_path,
                    record.node.start_line, record.node.end_line, record.node.summary,
                    record.node.content_hash, now, chunk_vec_blob,
                ],
            )?;
            conn.execute(
                "DELETE FROM fts_content WHERE node_id = ?1",
                params![record.node.id],
            )?;
            conn.execute(
                "INSERT INTO fts_content (node_id, project_id, name, content, file_path)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    record.node.id,
                    record.node.project_id,
                    record.node.name,
                    record.content,
                    record.node.file_path,
                ],
            )?;
            conn.execute(
                "INSERT OR IGNORE INTO edges (id, project_id, source_id, target_id, edge_type, weight)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    record.edge.id, record.edge.project_id,
                    record.edge.source_id, record.edge.target_id,
                    record.edge.edge_type.as_str(), record.edge.weight,
                ],
            )?;
            conn.execute(
                "INSERT OR REPLACE INTO file_hashes (file_path, project_id, content_hash, indexed_at)
                 VALUES (?1, ?2, ?3, datetime('now'))",
                params![record.hash_key, record.node.project_id, record.hash_value],
            )?;
        }
        Ok(())
    }

    pub fn get_node(&self, node_id: &str) -> Result<Option<Node>> {
        let conn = self.db.lock_ctx("get_node")?;
        let mut stmt = conn.prepare(
            "SELECT id, project_id, name, node_type, file_path, start_line, end_line, summary, content_hash
             FROM nodes WHERE id = ?1 AND project_id = ?2",
        )?;
        let result = stmt
            .query_row(params![node_id, self.project_id], |row| {
                crate::graph_queries::node_from_row(row)
            })
            .optional()
            .context("Failed to query node")?;
        Ok(result)
    }

    pub fn add_edge(&self, edge: &Edge) -> Result<()> {
        let conn = self.db.lock_ctx("add_edge")?;
        conn.execute(
            "INSERT OR IGNORE INTO edges (id, project_id, source_id, target_id, edge_type, weight)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                edge.id,
                edge.project_id,
                edge.source_id,
                edge.target_id,
                edge.edge_type.as_str(),
                edge.weight,
            ],
        )?;
        Ok(())
    }

    pub fn get_neighbors(&self, node_id: &str) -> Result<Vec<(Edge, Node)>> {
        let conn = self.db.lock_ctx("get_neighbors")?;
        let mut stmt = conn.prepare(
            "SELECT e.id, e.project_id, e.source_id, e.target_id, e.edge_type, e.weight,
                    n.id, n.project_id, n.name, n.node_type, n.file_path, n.start_line, n.end_line, n.summary, n.content_hash
             FROM edges e
             JOIN nodes n ON n.id = CASE WHEN e.source_id = ?1 THEN e.target_id ELSE e.source_id END
             WHERE (e.source_id = ?1 OR e.target_id = ?1) AND e.project_id = ?2",
        )?;
        let rows = stmt
            .query_map(params![node_id, self.project_id], |row| {
                Ok((
                    Edge {
                        id: row.get(0)?,
                        project_id: row.get(1)?,
                        source_id: row.get(2)?,
                        target_id: row.get(3)?,
                        edge_type: EdgeType::parse_str(&row.get::<_, String>(4)?),
                        weight: row.get(5)?,
                    },
                    Node {
                        id: row.get(6)?,
                        project_id: row.get(7)?,
                        name: row.get(8)?,
                        node_type: NodeType::parse_str(&row.get::<_, String>(9)?),
                        file_path: row.get(10)?,
                        start_line: row.get(11)?,
                        end_line: row.get(12)?,
                        summary: row.get(13)?,
                        content_hash: row.get(14)?,
                    },
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn index_fts(&self, node: &Node, content: &str) -> Result<()> {
        let conn = self.db.lock_ctx("index_fts")?;
        conn.execute(
            "DELETE FROM fts_content WHERE node_id = ?1",
            params![node.id],
        )?;
        conn.execute(
            "INSERT INTO fts_content (node_id, project_id, name, content, file_path)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![node.id, node.project_id, node.name, content, node.file_path,],
        )?;
        Ok(())
    }

    pub fn db(&self) -> &Arc<Mutex<Connection>> {
        &self.db
    }

    pub fn project_id(&self) -> &str {
        &self.project_id
    }

    pub fn create_node_builder(&self) -> NodeBuilder {
        NodeBuilder::new(&self.project_id)
    }

    pub fn create_edge_builder(&self) -> EdgeBuilder {
        EdgeBuilder::new(&self.project_id)
    }
}

fn node_vector_blob(node: &Node) -> Vec<u8> {
    let text = vector_ops::combined_node_text(node);
    let vec = neural_embed::embed(&text);
    vector_ops::vec_to_blob(&vec)
}

trait OptionalRow {
    fn optional(self) -> std::result::Result<Option<Node>, rusqlite::Error>;
}

impl OptionalRow for std::result::Result<Node, rusqlite::Error> {
    fn optional(self) -> std::result::Result<Option<Node>, rusqlite::Error> {
        match self {
            Ok(node) => Ok(Some(node)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}
