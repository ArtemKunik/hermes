// ChartApp/hermes-engine/src/graph_builders.rs
use crate::graph::{Edge, EdgeType, Node, NodeType};
use uuid::Uuid;

pub struct NodeBuilder {
    node: Node,
}

impl NodeBuilder {
    pub(crate) fn new(project_id: &str) -> Self {
        Self {
            node: Node {
                id: Uuid::new_v4().to_string(),
                project_id: project_id.to_string(),
                name: String::new(),
                node_type: NodeType::Concept,
                file_path: None,
                start_line: None,
                end_line: None,
                summary: None,
                content_hash: None,
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

    pub fn content_hash(mut self, hash: &str) -> Self {
        self.node.content_hash = Some(hash.to_string());
        self
    }

    pub fn build(self) -> Node {
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
                id: Uuid::new_v4().to_string(),
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

    pub fn build(self) -> Edge {
        self.edge
    }
}
