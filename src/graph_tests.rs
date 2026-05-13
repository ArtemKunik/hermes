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
