use crate::{
    graph::{Edge, EdgeType, KnowledgeGraph, Node, NodeType},
    HermesEngine,
};

fn make_graph(engine: &HermesEngine) -> KnowledgeGraph {
    KnowledgeGraph::new(engine.db().clone(), engine.project_id())
}

fn insert_node(graph: &KnowledgeGraph, id: &str, name: &str, file_path: &str) -> Node {
    let node = Node {
        id: id.to_string(),
        project_id: graph.project_id().to_string(),
        name: name.to_string(),
        node_type: NodeType::Function,
        file_path: Some(file_path.to_string()),
        start_line: Some(1),
        end_line: Some(10),
        summary: None,
        content_hash: None,
    };
    graph.add_node(&node).unwrap();
    node
}

// ── literal_search_by_name ───────────────────────────────────────────────

#[test]
fn literal_search_prefix_match() {
    let engine = HermesEngine::in_memory("gq-literal").unwrap();
    let graph = make_graph(&engine);
    insert_node(&graph, "n1", "fetch_alerts", "src/api.rs");
    insert_node(&graph, "n2", "process_alerts", "src/api.rs");

    let results = graph.literal_search_by_name("fetch").unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "fetch_alerts");
}

#[test]
fn literal_search_contains_fallback() {
    let engine = HermesEngine::in_memory("gq-contains").unwrap();
    let graph = make_graph(&engine);
    insert_node(&graph, "n1", "fetch_alerts_handler", "src/api.rs");

    let results = graph.literal_search_by_name("alerts").unwrap();
    assert!(!results.is_empty());
    assert_eq!(results[0].name, "fetch_alerts_handler");
}

#[test]
fn literal_search_is_case_insensitive() {
    let engine = HermesEngine::in_memory("gq-case").unwrap();
    let graph = make_graph(&engine);
    insert_node(&graph, "n1", "HandleRequest", "src/server.rs");

    let results = graph.literal_search_by_name("handlerequest").unwrap();
    assert_eq!(results.len(), 1);
}

#[test]
fn literal_search_preserves_unicode_fallback() {
    let engine = HermesEngine::in_memory("gq-unicode").unwrap();
    let graph = make_graph(&engine);
    insert_node(&graph, "n1", "RésuméHandler", "src/server.rs");

    let results = graph.literal_search_by_name("résumé").unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "RésuméHandler");
}

#[test]
fn literal_search_returns_empty_for_no_match() {
    let engine = HermesEngine::in_memory("gq-nomatch").unwrap();
    let graph = make_graph(&engine);
    insert_node(&graph, "n1", "my_func", "src/lib.rs");

    let results = graph.literal_search_by_name("nonexistent_xyz").unwrap();
    assert!(results.is_empty());
}

// ── get_all_nodes ────────────────────────────────────────────────────────────

#[test]
fn get_all_nodes_empty() {
    let engine = HermesEngine::in_memory("gq-allnodes-empty").unwrap();
    let graph = make_graph(&engine);
    assert!(graph.get_all_nodes().unwrap().is_empty());
}

#[test]
fn get_all_nodes_returns_inserted_nodes() {
    let engine = HermesEngine::in_memory("gq-allnodes").unwrap();
    let graph = make_graph(&engine);
    insert_node(&graph, "n1", "alpha", "src/a.rs");
    insert_node(&graph, "n2", "beta", "src/b.rs");

    let all = graph.get_all_nodes().unwrap();
    assert_eq!(all.len(), 2);
}

// ── get_all_file_paths ──────────────────────────────────────────────────────

#[test]
fn get_all_file_paths_only_returns_file_type_nodes() {
    let engine = HermesEngine::in_memory("gq-filepaths").unwrap();
    let graph = make_graph(&engine);

    let file_node = Node {
        id: "file-1".to_string(),
        project_id: graph.project_id().to_string(),
        name: "src/main.rs".to_string(),
        node_type: NodeType::File,
        file_path: Some("src/main.rs".to_string()),
        start_line: None,
        end_line: None,
        summary: None,
        content_hash: None,
    };
    graph.add_node(&file_node).unwrap();

    insert_node(&graph, "fn-1", "some_fn", "src/main.rs");

    let paths = graph.get_all_file_paths().unwrap();
    assert_eq!(paths.len(), 1);
    assert!(paths.contains("src/main.rs"));
}

// ── delete_nodes_for_file ─────────────────────────────────────────────────

#[test]
fn delete_nodes_for_file_removes_correct_nodes() {
    let engine = HermesEngine::in_memory("gq-delete").unwrap();
    let graph = make_graph(&engine);
    insert_node(&graph, "n1", "fn_a", "src/a.rs");
    insert_node(&graph, "n2", "fn_b", "src/b.rs");

    graph.delete_nodes_for_file("src/a.rs").unwrap();

    let all = graph.get_all_nodes().unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].name, "fn_b");
}

#[test]
fn delete_nodes_removes_associated_edges() {
    let engine = HermesEngine::in_memory("gq-delete-edges").unwrap();
    let graph = make_graph(&engine);
    let n1 = insert_node(&graph, "n1", "fn_a", "src/a.rs");
    let n2 = insert_node(&graph, "n2", "fn_b", "src/b.rs");

    let edge = Edge {
        id: "e1".to_string(),
        project_id: graph.project_id().to_string(),
        source_id: n1.id.clone(),
        target_id: n2.id.clone(),
        edge_type: EdgeType::Calls,
        weight: 1.0,
    };
    graph.add_edge(&edge).unwrap();

    graph.delete_nodes_for_file("src/a.rs").unwrap();

    let neighbors = graph.get_neighbors("n2").unwrap();
    assert!(neighbors.is_empty());
}

// ── fts_search ───────────────────────────────────────────────────────────────

#[test]
fn fts_search_finds_indexed_content() {
    let engine = HermesEngine::in_memory("gq-fts").unwrap();
    let graph = make_graph(&engine);
    let node = insert_node(&graph, "n1", "alerts_handler", "src/api.rs");
    graph
        .index_fts(&node, "handles incoming alert notifications")
        .unwrap();

    let results = graph.fts_search("\"alert\"", 10).unwrap();
    assert!(!results.is_empty());
    assert_eq!(results[0].0.id, "n1");
}

#[test]
fn fts_search_returns_empty_for_no_match() {
    let engine = HermesEngine::in_memory("gq-fts-empty").unwrap();
    let graph = make_graph(&engine);
    let node = insert_node(&graph, "n1", "handler", "src/api.rs");
    graph
        .index_fts(&node, "something completely different")
        .unwrap();

    let results = graph.fts_search("\"xyznonexistent\"", 10).unwrap();
    assert!(results.is_empty());
}

#[test]
fn fts_search_respects_limit() {
    let engine = HermesEngine::in_memory("gq-fts-limit").unwrap();
    let graph = make_graph(&engine);

    for i in 0..5 {
        let node = insert_node(
            &graph,
            &format!("n{i}"),
            &format!("handler_{i}"),
            "src/api.rs",
        );
        graph
            .index_fts(&node, "shared keyword present in content")
            .unwrap();
    }

    let results = graph.fts_search("\"shared\"", 3).unwrap();
    assert!(results.len() <= 3);
}
