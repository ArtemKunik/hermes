// ChartApp/hermes-engine/src/graph.rs
use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

pub use crate::graph_builders::{EdgeBuilder, NodeBuilder};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub node_type: NodeType,
    pub file_path: Option<String>,
    pub start_line: Option<i64>,
    pub end_line: Option<i64>,
    pub summary: Option<String>,
    pub content_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum NodeType {
    File,
    Module,
    Function,
    Struct,
    Impl,
    Trait,
    Enum,
    Concept,
    Document,
}

impl NodeType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Module => "module",
            Self::Function => "function",
            Self::Struct => "struct",
            Self::Impl => "impl",
            Self::Trait => "trait",
            Self::Enum => "enum",
            Self::Concept => "concept",
            Self::Document => "document",
        }
    }

    pub fn parse_str(s: &str) -> Self {
        match s {
            "file" => Self::File,
            "module" => Self::Module,
            "function" => Self::Function,
            "struct" => Self::Struct,
            "impl" => Self::Impl,
            "trait" => Self::Trait,
            "enum" => Self::Enum,
            "concept" => Self::Concept,
            "document" => Self::Document,
            _ => Self::Concept,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub id: String,
    pub project_id: String,
    pub source_id: String,
    pub target_id: String,
    pub edge_type: EdgeType,
    pub weight: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum EdgeType {
    Calls,
    Imports,
    Implements,
    DependsOn,
    Contains,
    Documents,
}

impl EdgeType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Calls => "calls",
            Self::Imports => "imports",
            Self::Implements => "implements",
            Self::DependsOn => "depends_on",
            Self::Contains => "contains",
            Self::Documents => "documents",
        }
    }

    pub fn parse_str(s: &str) -> Self {
        match s {
            "calls" => Self::Calls,
            "imports" => Self::Imports,
            "implements" => Self::Implements,
            "depends_on" => Self::DependsOn,
            "contains" => Self::Contains,
            "documents" => Self::Documents,
            _ => Self::DependsOn,
        }
    }
}

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
        let conn = self.db.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT OR REPLACE INTO nodes
             (id, project_id, name, node_type, file_path, start_line, end_line, summary, content_hash, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
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
                now,
            ],
        )?;
        Ok(())
    }

    pub fn get_node(&self, node_id: &str) -> Result<Option<Node>> {
        let conn = self.db.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut stmt = conn.prepare(
            "SELECT id, project_id, name, node_type, file_path, start_line, end_line, summary, content_hash
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
                })
            })
            .optional()
            .context("Failed to query node")?;
        Ok(result)
    }

    pub fn add_edge(&self, edge: &Edge) -> Result<()> {
        let conn = self.db.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
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
        let conn = self.db.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
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
        let conn = self.db.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
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
