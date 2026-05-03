use super::*;
use crate::graph::KnowledgeGraph;
use crate::HermesEngine;
use tempfile::TempDir;

fn make_graph_for(engine: &HermesEngine) -> KnowledgeGraph {
    KnowledgeGraph::new(engine.db().clone(), engine.project_id())
}

#[test]
fn test_ingest_empty_dir_returns_zero_report() {
    let dir = TempDir::new().unwrap();
    let engine = HermesEngine::in_memory("test-ingest").unwrap();
    let graph = make_graph_for(&engine);
    let pipeline = IngestionPipeline::new(&graph);
    let report = pipeline.ingest_directory(dir.path()).unwrap();
    assert_eq!(report.total_files, 0);
    assert_eq!(report.nodes_created, 0);
}

#[test]
fn test_unchanged_file_is_skipped_on_reindex() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("test.rs");
    std::fs::write(&file, "fn main() {}").unwrap();

    let engine = HermesEngine::in_memory("test-skip").unwrap();
    let graph = make_graph_for(&engine);
    let pipeline = IngestionPipeline::new(&graph);

    let report1 = pipeline.ingest_directory(dir.path()).unwrap();
    assert_eq!(report1.indexed, 1);
    assert_eq!(report1.skipped, 0);

    let report2 = pipeline.ingest_directory(dir.path()).unwrap();
    assert_eq!(report2.indexed, 0);
    assert_eq!(report2.skipped, 1);
}

#[test]
fn test_ingest_non_utf8_file_succeeds() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("latin1.md");
    std::fs::write(&file, b"caf\xe9 and r\xe9sum\xe9").unwrap();

    let engine = HermesEngine::in_memory("test-non-utf8").unwrap();
    let graph = make_graph_for(&engine);
    let pipeline = IngestionPipeline::new(&graph);
    let report = pipeline.ingest_directory(dir.path()).unwrap();

    assert_eq!(report.errors, 0, "non-UTF-8 file must not be counted as an error");
    assert_eq!(report.indexed, 1);
}

#[test]
fn test_ingest_stores_content_tokens_on_nodes() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("tokens_test.rs");
    let content = r#"fn one() { println!(\"one\"); }
fn two() { println!(\"two\"); }
"#;
    std::fs::write(&file, content).unwrap();

    let engine = HermesEngine::in_memory("test-tokens").unwrap();
    let graph = make_graph_for(&engine);
    let pipeline = IngestionPipeline::new(&graph);

    pipeline.ingest_directory(dir.path()).unwrap();

    let nodes = graph.get_all_nodes().unwrap();
    let any_with_tokens = nodes.iter().any(|n| n.content_tokens.unwrap_or(0) > 0);
    assert!(any_with_tokens, "Ingested nodes should have content_tokens set");

    let file_node = nodes
        .iter()
        .find(|n| n.file_path.as_deref() == Some(&file.to_string_lossy().to_string()));
    if let Some(f) = file_node {
        let est = crate::search::estimate_tokens(content);
        assert_eq!(f.content_tokens.unwrap_or(0), est);
    }
}

#[test]
fn test_backfill_populates_missing_content_tokens() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("backfill.rs");
    let content = "fn a() {}\nfn b() {}\n";
    std::fs::write(&file, content).unwrap();

    let engine = HermesEngine::in_memory("test-backfill").unwrap();
    let graph = make_graph_for(&engine);

    let node = graph
        .create_node_builder()
        .name("backfill_chunk")
        .node_type(crate::graph::NodeType::Function)
        .file_path(&file.to_string_lossy())
        .lines(1, 2)
        .summary("backfill test")
        .build();
    graph.add_node(&node).unwrap();

    let pipeline = IngestionPipeline::new(&graph);
    let updated = pipeline.backfill_content_tokens().unwrap();
    assert_eq!(updated, 1, "One node should have been updated");

    let fetched = graph.get_node(&node.id).unwrap().unwrap();
    let est = crate::search::estimate_tokens(content);
    assert_eq!(fetched.content_tokens.unwrap_or(0), est);
}

#[test]
fn test_ingest_writes_symbol_embeddings() {
    std::env::remove_var("OPENAI_API_KEY");

    let dir = TempDir::new().unwrap();
    let file = dir.path().join("foo.rs");
    let content = "fn myfunc() { }";
    std::fs::write(&file, content).unwrap();

    let engine = HermesEngine::in_memory("test-embed").unwrap();
    let graph = make_graph_for(&engine);
    let pipeline = IngestionPipeline::new(&graph);
    pipeline.ingest_directory(dir.path()).unwrap();

    let conn = engine.db().lock().unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM symbol_embeddings", [], |r| r.get(0))
        .unwrap();
    assert!(count >= 1, "expected embeddings table to be populated");
}

#[test]
fn test_stale_file_removed_after_deletion() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("will_be_deleted.rs");
    std::fs::write(&file, "fn foo() {}").unwrap();

    let engine = HermesEngine::in_memory("test-stale").unwrap();
    let graph = make_graph_for(&engine);
    let pipeline = IngestionPipeline::new(&graph);

    pipeline.ingest_directory(dir.path()).unwrap();
    let paths_after_first = graph.get_all_file_paths().unwrap();
    assert!(!paths_after_first.is_empty());

    std::fs::remove_file(&file).unwrap();
    pipeline.ingest_directory(dir.path()).unwrap();
    let paths_after_second = graph.get_all_file_paths().unwrap();
    assert!(paths_after_second.is_empty());
}
