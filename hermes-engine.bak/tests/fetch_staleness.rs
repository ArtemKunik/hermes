use hermes_engine::{
    graph::{KnowledgeGraph, NodeType},
    ingestion::hash_tracker::compute_hash,
    search::SearchEngine,
    HermesEngine,
};
use tempfile::NamedTempFile;

fn add_node_for_file(
    graph: &KnowledgeGraph,
    file_path: &str,
    start_line: i64,
    end_line: i64,
    content_hash: &str,
) -> String {
    let node = graph
        .create_node_builder()
        .name("test_node")
        .node_type(NodeType::Function)
        .file_path(file_path)
        .lines(start_line, end_line)
        .content_hash(content_hash)
        .build();
    let node_id = node.id.clone();
    graph.add_node(&node).unwrap();
    node_id
}

#[test]
fn test_fetch_reports_fresh_when_content_hash_matches() {
    let file = NamedTempFile::new().unwrap();
    std::fs::write(file.path(), "fn alpha() {}\nfn beta() {}\n").unwrap();

    let line_content = "fn alpha() {}";
    let stored_hash = compute_hash(line_content);

    let engine = HermesEngine::in_memory("fetch-fresh").unwrap();
    let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
    let node_id = add_node_for_file(&graph, &file.path().to_string_lossy(), 1, 1, &stored_hash);

    let search = SearchEngine::new(&graph, engine.search_cache());
    let response = search.fetch(&node_id).unwrap().unwrap();

    assert!(!response.is_stale);
    assert_eq!(response.stored_content_hash, Some(stored_hash.clone()));
    assert_eq!(response.current_content_hash, Some(stored_hash));
}

#[test]
fn test_fetch_marks_stale_when_content_hash_mismatch() {
    let file = NamedTempFile::new().unwrap();
    std::fs::write(file.path(), "fn gamma() {}\n").unwrap();

    let engine = HermesEngine::in_memory("fetch-stale-hash").unwrap();
    let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
    let node_id = add_node_for_file(
        &graph,
        &file.path().to_string_lossy(),
        1,
        1,
        &compute_hash("fn old_gamma() {}"),
    );

    let search = SearchEngine::new(&graph, engine.search_cache());
    let response = search.fetch(&node_id).unwrap().unwrap();

    assert!(response.is_stale);
    assert_eq!(
        response.current_content_hash,
        Some(compute_hash("fn gamma() {}"))
    );
    assert!(response.stale_reason.unwrap_or_default().contains("hash"));
}

#[test]
fn test_fetch_marks_stale_when_file_is_missing() {
    let file = NamedTempFile::new().unwrap();
    let missing_path = file.path().to_string_lossy().to_string();
    std::fs::write(&missing_path, "fn delta() {}\n").unwrap();
    let stored_hash = compute_hash("fn delta() {}");
    std::fs::remove_file(&missing_path).unwrap();

    let engine = HermesEngine::in_memory("fetch-stale-missing").unwrap();
    let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
    let node_id = add_node_for_file(&graph, &missing_path, 1, 1, &stored_hash);

    let search = SearchEngine::new(&graph, engine.search_cache());
    let response = search.fetch(&node_id).unwrap().unwrap();

    assert!(response.is_stale);
    assert!(response.content.starts_with("[File not found: "));
    assert!(response
        .stale_reason
        .unwrap_or_default()
        .contains("missing"));
}
