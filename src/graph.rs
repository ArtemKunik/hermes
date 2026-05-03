// ChartApp/hermes-engine/src/graph.rs
use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{params, Connection};
use std::sync::{Arc, Mutex};

pub use crate::graph_builders::{EdgeBuilder, NodeBuilder};
pub use crate::graph_types::{Edge, EdgeType, Node, NodeType};
pub use crate::graph_support::{f32_slice_to_blob, blob_to_f32_vector, OptionalRow};

pub struct KnowledgeGraph {
    db: GraphConn,
    project_id: String,
}

enum GraphConn {
    Shared(Arc<Mutex<Connection>>),
    Borrowed(*const Connection),
}

// Safety: KnowledgeGraph is used in threads, but we ensure the lifetime of
// the borrowed connection exceeds the graph during execute_tool_call.
unsafe impl Send for GraphConn {}
unsafe impl Sync for GraphConn {}

impl KnowledgeGraph {
    pub fn new(db: Arc<Mutex<Connection>>, project_id: &str) -> Self {
        Self {
            db: GraphConn::Shared(db),
            project_id: project_id.to_string(),
        }
    }

    /// TRACK-066: Create a graph instance from a raw connection (read-only isolation).
    pub fn from_conn(conn: &Connection, project_id: &str) -> Self {
        Self {
            db: GraphConn::Borrowed(conn as *const Connection),
            project_id: project_id.to_string(),
        }
    }

    pub fn with_conn<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&Connection) -> Result<R>,
    {
        match &self.db {
            GraphConn::Shared(arc) => {
                let conn = arc.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
                f(&conn)
            }
            GraphConn::Borrowed(ptr) => {
                // Safety: ptr is valid for the duration of the tool call.
                let conn = unsafe { &**ptr };
                f(conn)
            }
        }
    }

    pub fn db(&self) -> &Arc<Mutex<Connection>> {
        match &self.db {
            GraphConn::Shared(arc) => arc,
            GraphConn::Borrowed(_) => panic!("KnowledgeGraph::db() called on borrowed connection"),
        }
    }

    /// TRACK-066: Execute a closure with a raw connection, works for both Shared and Borrowed modes.
    pub fn with_raw_conn<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&Connection) -> Result<R>,
    {
        self.with_conn(f)
    }

    pub fn add_node(&self, node: &Node) -> Result<()> {
        self.with_conn(|conn| {
            let now = Utc::now().to_rfc3339();
            conn.execute(
                "INSERT OR REPLACE INTO nodes
                 (id, project_id, name, node_type, file_path, start_line, end_line, summary, content_hash, content_tokens, object_type, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    node.id,
                    node.project_id,
                    node.name,
                    node.node_type.as_str(),
                    node.file_path,
                    node.start_line,
                    node.end_line,
                    node.summary,
                    node.content_hash,
                    node.content_tokens.map(|v| v as i64),
                    node.object_type,
                    now,
                ],
            )?;
            Ok(())
        })
    }

    pub fn get_node(&self, node_id: &str) -> Result<Option<Node>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, project_id, name, node_type, file_path, start_line, end_line, summary, content_hash, content_tokens, object_type
                 FROM nodes WHERE id = ?1 AND project_id = ?2",
            )?;
            let result = stmt
                .query_row(params![node_id, self.project_id], |row| {
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
                        content_tokens: row.get::<_, Option<i64>>(9)?.map(|v| v as u64),
                        object_type: row.get(10)?,
                    })
                })
                .optional()
                .context("Failed to query node")?;
            Ok(result)
        })
    }

    pub fn add_edge(&self, edge: &Edge) -> Result<()> {
        self.with_conn(|conn| {
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
        })
    }

    pub fn project_id(&self) -> &str {
        &self.project_id
    }

    // ------------------------------------------------------------------
    // Symbol embedding helpers (for duplicate detection scanner)
    // ------------------------------------------------------------------

    /// Insert or update a symbol embedding record in the database.
    pub fn upsert_symbol_embedding(
        &self,
        id: &str,
        symbol_name: &str,
        file_path: &str,
        language: &str,
        signature: &str,
        snippet: &str,
        embedding: &[f32],
    ) -> Result<()> {
        self.with_conn(|conn| {
            let blob = f32_slice_to_blob(embedding);
            conn.execute(
                "INSERT OR REPLACE INTO symbol_embeddings \
                 (id, symbol_name, file_path, language, signature, snippet, embedding) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![id, symbol_name, file_path, language, signature, snippet, blob],
            )?;
            Ok(())
        })
    }

    /// Return all stored embeddings along with their metadata.  The returned tuple is
    /// (symbol_name, file_path, signature, snippet, embedding_vec).
    pub fn get_all_symbol_embeddings(&self) -> Result<Vec<(String, String, String, String, Vec<f32>)>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT symbol_name, file_path, signature, snippet, embedding \
                 FROM symbol_embeddings",
            )?;
            let mut rows = stmt.query([])?;
            let mut results = Vec::new();
            while let Some(row) = rows.next()? {
                let symbol: String = row.get(0)?;
                let file: String = row.get(1)?;
                let signature: String = row.get(2)?;
                let snippet: String = row.get(3)?;
                let blob: Vec<u8> = row.get(4)?;
                let embedding = blob_to_f32_vector(&blob)?;
                results.push((symbol, file, signature, snippet, embedding));
            }
            Ok(results)
        })
    }

    pub fn create_node_builder(&self) -> NodeBuilder {
        NodeBuilder::new(&self.project_id)
    }

    pub fn create_edge_builder(&self) -> EdgeBuilder {
        EdgeBuilder::new(&self.project_id)
    }

    /// Update the stored `content_tokens` for a node.
    pub fn update_node_content_tokens(&self, node_id: &str, tokens: u64) -> Result<()> {
        self.with_conn(|conn| {
            let now = Utc::now().to_rfc3339();
            conn.execute(
                "UPDATE nodes SET content_tokens = ?1, updated_at = ?2 WHERE id = ?3 AND project_id = ?4",
                params![tokens as i64, now, node_id, self.project_id()],
            )?;
            Ok(())
        })
    }
}
