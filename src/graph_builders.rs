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

#[cfg(test)]
mod tests {
    use super::*;

    // ── NodeBuilder ───────────────────────────────────────────────────────

    #[test]
    fn node_builder_defaults() {
        let node = NodeBuilder::new("proj-1").build();
        assert_eq!(node.project_id, "proj-1");
        assert!(node.name.is_empty());
        assert_eq!(node.node_type, NodeType::Concept);
        assert!(node.file_path.is_none());
        assert!(node.start_line.is_none());
        assert!(node.end_line.is_none());
        assert!(node.summary.is_none());
        assert!(node.content_hash.is_none());
        // id is a new uuid each time
        assert!(!node.id.is_empty());
    }

    #[test]
    fn node_builder_all_fields() {
        let node = NodeBuilder::new("proj")
            .name("my_fn")
            .node_type(NodeType::Function)
            .file_path("src/lib.rs")
            .lines(5, 30)
            .summary("does things")
            .content_hash("deadbeef")
            .build();

        assert_eq!(node.name, "my_fn");
        assert_eq!(node.node_type, NodeType::Function);
        assert_eq!(node.file_path.as_deref(), Some("src/lib.rs"));
        assert_eq!(node.start_line, Some(5));
        assert_eq!(node.end_line, Some(30));
        assert_eq!(node.summary.as_deref(), Some("does things"));
        assert_eq!(node.content_hash.as_deref(), Some("deadbeef"));
    }

    #[test]
    fn node_builder_produces_unique_ids() {
        let a = NodeBuilder::new("p").build();
        let b = NodeBuilder::new("p").build();
        assert_ne!(a.id, b.id);
    }

    // ── EdgeBuilder ───────────────────────────────────────────────────────

    #[test]
    fn edge_builder_defaults() {
        let edge = EdgeBuilder::new("proj").build();
        assert_eq!(edge.project_id, "proj");
        assert!(edge.source_id.is_empty());
        assert!(edge.target_id.is_empty());
        assert_eq!(edge.edge_type, EdgeType::DependsOn);
        assert!((edge.weight - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn edge_builder_all_fields() {
        let edge = EdgeBuilder::new("proj")
            .source("n1")
            .target("n2")
            .edge_type(EdgeType::Calls)
            .weight(0.75)
            .build();

        assert_eq!(edge.source_id, "n1");
        assert_eq!(edge.target_id, "n2");
        assert_eq!(edge.edge_type, EdgeType::Calls);
        assert!((edge.weight - 0.75).abs() < f64::EPSILON);
    }

    #[test]
    fn edge_builder_produces_unique_ids() {
        let a = EdgeBuilder::new("p").build();
        let b = EdgeBuilder::new("p").build();
        assert_ne!(a.id, b.id);
    }
}
