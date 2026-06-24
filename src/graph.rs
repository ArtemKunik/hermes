use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{params, Connection};
use std::sync::{Arc, Mutex};

pub use crate::graph_builders::{EdgeBuilder, NodeBuilder};
pub use crate::graph_support::{blob_to_f32_vector, f32_slice_to_blob, OptionalRow};
pub use crate::graph_types::{Edge, EdgeType, Node, NodeType};

pub struct KnowledgeGraph {
    db: GraphConn,
    project_id: String,
}

enum GraphConn {
    Shared(Arc<Mutex<Connection>>),
    Borrowed(*const Connection),
}

unsafe impl Send for GraphConn {}
unsafe impl Sync for GraphConn {}

impl KnowledgeGraph {
    pub fn new(db: Arc<Mutex<Connection>>, project_id: &str) -> Self {
        Self {
            db: GraphConn::Shared(db),
            project_id: project_id.to_string(),
        }
    }

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

    pub fn with_raw_conn<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&Connection) -> Result<R>,
    {
        self.with_conn(f)
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

    pub fn update_node_content_tokens(&self, node_id: &str, tokens: u64) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE nodes SET content_tokens = ?1 WHERE id = ?2 AND project_id = ?3",
                params![tokens as i64, node_id, self.project_id],
            )?;
            Ok(())
        })
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
            content_hash: None,
            content_tokens: None,
            object_type: None,
        }
    }

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
            NodeType::Interface,
            NodeType::Concept,
            NodeType::Document,
            NodeType::Config,
        ];
        for v in &variants {
            assert_eq!(&NodeType::parse_str(v.as_str()), v);
        }
    }

    #[test]
    fn edge_type_roundtrip() {
        let variants = [
            EdgeType::Calls,
            EdgeType::Imports,
            EdgeType::Implements,
            EdgeType::DependsOn,
            EdgeType::Contains,
            EdgeType::Documents,
            EdgeType::Defines,
            EdgeType::Uses,
            EdgeType::Tests,
        ];
        for v in &variants {
            assert_eq!(&EdgeType::parse_str(v.as_str()), v);
        }
    }

    #[test]
    fn add_and_get_node_roundtrip() {
        let engine = HermesEngine::in_memory("graph-crud").unwrap();
        let graph = make_graph(&engine);
        let node = sample_node(engine.project_id());
        graph.add_node(&node).unwrap();

        let fetched = graph.get_node("node-1").unwrap().expect("node must exist");
        assert_eq!(fetched.name, "my_function");
        assert_eq!(fetched.node_type, NodeType::Function);
    }

    #[test]
    fn get_node_returns_none_for_missing_id() {
        let engine = HermesEngine::in_memory("graph-missing").unwrap();
        let graph = make_graph(&engine);
        let result = graph.get_node("nonexistent").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn add_edge_and_get_neighbors() {
        let engine = HermesEngine::in_memory("graph-edge").unwrap();
        let graph = make_graph(&engine);

        for id in ["n1", "n2"] {
            graph.add_node(&Node {
                id: id.to_string(),
                project_id: engine.project_id().to_string(),
                name: id.to_string(),
                node_type: NodeType::Function,
                file_path: None,
                start_line: None,
                end_line: None,
                summary: None,
                content_hash: None,
                content_tokens: None,
                object_type: None,
            }).unwrap();
        }

        graph.add_edge(&Edge {
            id: "e1".to_string(),
            project_id: engine.project_id().to_string(),
            source_id: "n1".to_string(),
            target_id: "n2".to_string(),
            edge_type: EdgeType::Calls,
            weight: 1.0,
        }).unwrap();

        let neighbors = graph.get_neighbors("n1").unwrap();
        assert_eq!(neighbors.len(), 1);
        assert_eq!(neighbors[0].1.name, "n2");
    }

    #[test]
    fn index_fts_stores_and_replaces_content() {
        let engine = HermesEngine::in_memory("graph-fts").unwrap();
        let graph = make_graph(&engine);
        let node = sample_node(engine.project_id());
        graph.add_node(&node).unwrap();
        graph.index_fts(&node, "initial content").unwrap();
        graph.index_fts(&node, "updated content").unwrap();

        let results = graph.fts_search("\"updated\"", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0.id, "node-1");
    }

    #[test]
    fn update_content_tokens() {
        let engine = HermesEngine::in_memory("graph-tokens").unwrap();
        let graph = make_graph(&engine);
        let node = sample_node(engine.project_id());
        graph.add_node(&node).unwrap();
        graph.update_node_content_tokens("node-1", 500).unwrap();

        let fetched = graph.get_node("node-1").unwrap().unwrap();
        assert_eq!(fetched.content_tokens, Some(500));
    }
}
