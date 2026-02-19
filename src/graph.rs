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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HermesEngine;

    fn make_graph(engine: &HermesEngine) -> KnowledgeGraph {
        KnowledgeGraph::new(engine.db().clone(), engine.project_id())
    }

    fn sample_node(project_id: &str) -> Node {
        Node {
            id: "node-1".to_string(),
            project_id: project_id.to_string(),
            name: "my_function".to_string(),
            node_type: NodeType::Function,
            file_path: Some("src/lib.rs".to_string()),
            start_line: Some(10),
            end_line: Some(20),
            summary: Some("Does something".to_string()),
            content_hash: Some("abc123".to_string()),
        }
    }

    // ── NodeType ───────────────────────────────────────────────────────────

    #[test]
    fn node_type_roundtrip() {
        let variants = [
            NodeType::File,
            NodeType::Module,
            NodeType::Function,
            NodeType::Struct,
            NodeType::Impl,
            NodeType::Trait,
            NodeType::Enum,
            NodeType::Concept,
            NodeType::Document,
        ];
        for v in &variants {
            assert_eq!(&NodeType::parse_str(v.as_str()), v);
        }
    }

    #[test]
    fn node_type_unknown_falls_back_to_concept() {
        assert_eq!(NodeType::parse_str("mystery"), NodeType::Concept);
    }

    // ── EdgeType ───────────────────────────────────────────────────────────

    #[test]
    fn edge_type_roundtrip() {
        let variants = [
            EdgeType::Calls,
            EdgeType::Imports,
            EdgeType::Implements,
            EdgeType::DependsOn,
            EdgeType::Contains,
            EdgeType::Documents,
        ];
        for v in &variants {
            assert_eq!(&EdgeType::parse_str(v.as_str()), v);
        }
    }

    #[test]
    fn edge_type_unknown_falls_back_to_depends_on() {
        assert_eq!(EdgeType::parse_str("blah"), EdgeType::DependsOn);
    }

    // ── KnowledgeGraph CRUD ───────────────────────────────────────────────

    #[test]
    fn add_and_get_node_roundtrip() {
        let engine = HermesEngine::in_memory("graph-crud").unwrap();
        let graph = make_graph(&engine);
        let node = sample_node(engine.project_id());
        graph.add_node(&node).unwrap();

        let fetched = graph.get_node("node-1").unwrap().expect("node must exist");
        assert_eq!(fetched.name, "my_function");
        assert_eq!(fetched.node_type, NodeType::Function);
        assert_eq!(fetched.start_line, Some(10));
        assert_eq!(fetched.summary.as_deref(), Some("Does something"));
    }

    #[test]
    fn get_node_returns_none_for_missing_id() {
        let engine = HermesEngine::in_memory("graph-missing").unwrap();
        let graph = make_graph(&engine);
        let result = graph.get_node("nonexistent").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn add_node_is_idempotent_with_replace() {
        let engine = HermesEngine::in_memory("graph-replace").unwrap();
        let graph = make_graph(&engine);
        let node = sample_node(engine.project_id());
        graph.add_node(&node).unwrap();

        let mut updated = node.clone();
        updated.name = "renamed_function".to_string();
        graph.add_node(&updated).unwrap();

        let fetched = graph.get_node("node-1").unwrap().unwrap();
        assert_eq!(fetched.name, "renamed_function");
    }

    #[test]
    fn add_edge_and_get_neighbors() {
        let engine = HermesEngine::in_memory("graph-edge").unwrap();
        let graph = make_graph(&engine);

        let n1 = Node {
            id: "n1".to_string(),
            project_id: engine.project_id().to_string(),
            name: "caller".to_string(),
            node_type: NodeType::Function,
            file_path: None,
            start_line: None,
            end_line: None,
            summary: None,
            content_hash: None,
        };
        let n2 = Node {
            id: "n2".to_string(),
            project_id: engine.project_id().to_string(),
            name: "callee".to_string(),
            node_type: NodeType::Function,
            file_path: None,
            start_line: None,
            end_line: None,
            summary: None,
            content_hash: None,
        };
        graph.add_node(&n1).unwrap();
        graph.add_node(&n2).unwrap();

        let edge = Edge {
            id: "e1".to_string(),
            project_id: engine.project_id().to_string(),
            source_id: "n1".to_string(),
            target_id: "n2".to_string(),
            edge_type: EdgeType::Calls,
            weight: 1.0,
        };
        graph.add_edge(&edge).unwrap();

        let neighbors_of_n1 = graph.get_neighbors("n1").unwrap();
        assert_eq!(neighbors_of_n1.len(), 1);
        assert_eq!(neighbors_of_n1[0].1.name, "callee");
        assert_eq!(neighbors_of_n1[0].0.edge_type, EdgeType::Calls);

        let neighbors_of_n2 = graph.get_neighbors("n2").unwrap();
        assert_eq!(neighbors_of_n2.len(), 1);
        assert_eq!(neighbors_of_n2[0].1.name, "caller");
    }

    #[test]
    fn add_edge_ignore_duplicates() {
        let engine = HermesEngine::in_memory("graph-edge-dup").unwrap();
        let graph = make_graph(&engine);

        for id in ["na", "nb"] {
            graph
                .add_node(&Node {
                    id: id.to_string(),
                    project_id: engine.project_id().to_string(),
                    name: id.to_string(),
                    node_type: NodeType::Concept,
                    file_path: None,
                    start_line: None,
                    end_line: None,
                    summary: None,
                    content_hash: None,
                })
                .unwrap();
        }

        let edge = Edge {
            id: "dup-e".to_string(),
            project_id: engine.project_id().to_string(),
            source_id: "na".to_string(),
            target_id: "nb".to_string(),
            edge_type: EdgeType::Imports,
            weight: 1.0,
        };
        graph.add_edge(&edge).unwrap();
        graph.add_edge(&edge).unwrap(); // should not panic

        assert_eq!(graph.get_neighbors("na").unwrap().len(), 1);
    }

    #[test]
    fn index_fts_stores_and_replaces_content() {
        let engine = HermesEngine::in_memory("graph-fts").unwrap();
        let graph = make_graph(&engine);
        let node = sample_node(engine.project_id());
        graph.add_node(&node).unwrap();

        graph.index_fts(&node, "initial content").unwrap();
        // Second call should replace without error
        graph.index_fts(&node, "updated content").unwrap();

        // Verify via raw FTS query returns one row
        let results = graph.fts_search("\"updated\"", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0.id, "node-1");
    }

    #[test]
    fn no_neighbors_for_isolated_node() {
        let engine = HermesEngine::in_memory("graph-isolated").unwrap();
        let graph = make_graph(&engine);
        let node = sample_node(engine.project_id());
        graph.add_node(&node).unwrap();
        assert!(graph.get_neighbors("node-1").unwrap().is_empty());
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
