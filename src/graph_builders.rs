// ChartApp/hermes-engine/src/graph_builders.rs
use crate::graph::{Edge, EdgeType, Node, NodeType};
use sha2::{Digest, Sha256};

/// Generate a deterministic node ID from a node's stable attributes.
/// Uses SHA-256(file_path + node_type + symbol_signature) so re-indexing
/// produces the same ID, preserving reinforcement/decay weights across runs.
pub fn deterministic_node_id(
    project_id: &str,
    file_path: &str,
    node_type: &str,
    name: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(project_id.as_bytes());
    hasher.update(b"::");
    hasher.update(file_path.as_bytes());
    hasher.update(b"::");
    hasher.update(node_type.as_bytes());
    hasher.update(b"::");
    hasher.update(name.as_bytes());
    hex::encode(hasher.finalize())
}

pub struct NodeBuilder {
    node: Node,
}

impl NodeBuilder {
    pub(crate) fn new(project_id: &str) -> Self {
        Self {
            node: Node {
                id: String::new(),
                project_id: project_id.to_string(),
                name: String::new(),
                node_type: NodeType::Concept,
                file_path: None,
                start_line: None,
                end_line: None,
                summary: None,
                content_hash: None,
                content_tokens: None,
                object_type: None,
            },
        }
    }

    pub fn name(mut self, name: &str) -> Self {
        self.node.name = name.to_string();
        self
    }

    pub fn node_type(mut self, node_type: NodeType) -> Self {
        self.node.node_type = node_type;
        self
    }

    pub fn file_path(mut self, path: &str) -> Self {
        self.node.file_path = Some(path.to_string());
        self
    }

    pub fn lines(mut self, start: i64, end: i64) -> Self {
        self.node.start_line = Some(start);
        self.node.end_line = Some(end);
        self
    }

    pub fn summary(mut self, summary: &str) -> Self {
        self.node.summary = Some(summary.to_string());
        self
    }

    pub fn object_type(mut self, object_type: &str) -> Self {
        self.node.object_type = Some(object_type.to_string());
        self
    }

    pub fn content_tokens(mut self, tokens: u64) -> Self {
        self.node.content_tokens = Some(tokens);
        self
    }

    pub fn build(mut self) -> Node {
        if self.node.id.is_empty() {
            self.node.id = deterministic_node_id(
                &self.node.project_id,
                self.node.file_path.as_deref().unwrap_or("unknown"),
                self.node.node_type.as_str(),
                &self.node.name,
            );
        }
        self.node
    }
}

pub struct EdgeBuilder {
    edge: Edge,
}

impl EdgeBuilder {
    pub(crate) fn new(project_id: &str) -> Self {
        Self {
            edge: Edge {
                id: String::new(),
                project_id: project_id.to_string(),
                source_id: String::new(),
                target_id: String::new(),
                edge_type: EdgeType::DependsOn,
                weight: 1.0,
            },
        }
    }

    pub fn source(mut self, source_id: &str) -> Self {
        self.edge.source_id = source_id.to_string();
        self
    }

    pub fn target(mut self, target_id: &str) -> Self {
        self.edge.target_id = target_id.to_string();
        self
    }

    pub fn edge_type(mut self, edge_type: EdgeType) -> Self {
        self.edge.edge_type = edge_type;
        self
    }

    pub fn weight(mut self, weight: f64) -> Self {
        self.edge.weight = weight;
        self
    }

    pub fn build(mut self) -> Edge {
        if self.edge.id.is_empty() {
            let mut hasher = Sha256::new();
            hasher.update(self.edge.project_id.as_bytes());
            hasher.update(b"::");
            hasher.update(self.edge.source_id.as_bytes());
            hasher.update(b"::");
            hasher.update(self.edge.target_id.as_bytes());
            hasher.update(b"::");
            hasher.update(self.edge.edge_type.as_str().as_bytes());
            self.edge.id = hex::encode(hasher.finalize());
        }
        self.edge
    }
}
