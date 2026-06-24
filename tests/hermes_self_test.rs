use std::path::Path;
use std::time::Duration;

use hermes_engine::accounting::Accountant;
use hermes_engine::graph::KnowledgeGraph;
use hermes_engine::ingestion::IngestionPipeline;
use hermes_engine::pointer::FetchResponse;
use hermes_engine::search::{SearchEngine, SearchMode};
use hermes_engine::temporal::TemporalStore;
use hermes_engine::temporal_types::{AddFactInput, FactFilter, FactType};
use hermes_engine::weight::WeightStore;
use hermes_engine::HermesEngine;

fn copy_source_to(dir: &Path) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    collect_and_copy(&src, &src, dir);
}

fn collect_and_copy(root: &Path, current: &Path, dest_root: &Path) {
    if let Ok(entries) = std::fs::read_dir(current) {
        for entry in entries {
            let Ok(entry) = entry else { continue };
            if entry.file_type().map_or(false, |t| t.is_dir()) {
                collect_and_copy(root, &entry.path(), dest_root);
            } else if entry.path().extension().map_or(false, |ext| ext == "rs") {
                let path = entry.path();
                let rel = path.strip_prefix(root).unwrap();
                let dest = dest_root.join(rel);
                if let Some(parent) = dest.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::copy(entry.path(), &dest);
            }
        }
    }
}

#[test]
fn self_index_indexes_all_source_files() {
    let engine = HermesEngine::in_memory("self-index").unwrap();
    let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
    let pipeline = IngestionPipeline::new(&graph);

    let dir = tempfile::TempDir::new().unwrap();
    copy_source_to(dir.path());

    let report = pipeline.ingest_directory(dir.path()).unwrap();
    assert!(report.indexed > 0, "must have indexed some files");
    assert!(report.nodes_created > 0, "must have created nodes");
    assert_eq!(report.errors, 0, "ingestion must have zero errors");

    let all = graph.get_all_nodes().unwrap();
    assert!(all.len() >= report.nodes_created);
}

#[test]
fn self_search_finds_knowledge_graph_struct() {
    let engine = HermesEngine::in_memory("self-search-struct").unwrap();
    let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
    let pipeline = IngestionPipeline::new(&graph);

    let dir = tempfile::TempDir::new().unwrap();
    copy_source_to(dir.path());
    pipeline.ingest_directory(dir.path()).unwrap();

    let search = SearchEngine::new(&graph, engine.search_cache());
    let response = search.search("KnowledgeGraph", 10, &SearchMode::Smart).unwrap();

    assert!(
        !response.pointers.is_empty(),
        "must find KnowledgeGraph in its own source"
    );
    let ids: Vec<&str> = response.pointers.iter().map(|p| p.id.as_str()).collect();
    println!("  KnowledgeGraph pointers: {ids:?}");
}

#[test]
fn self_search_finds_fn_in_memory() {
    let engine = HermesEngine::in_memory("self-search-fn").unwrap();
    let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
    let pipeline = IngestionPipeline::new(&graph);

    let dir = tempfile::TempDir::new().unwrap();
    copy_source_to(dir.path());
    pipeline.ingest_directory(dir.path()).unwrap();

    let search = SearchEngine::new(&graph, engine.search_cache());
    let response = search
        .search("in_memory", 10, &SearchMode::Smart)
        .unwrap();

    assert!(
        !response.pointers.is_empty(),
        "must find in_memory function definition"
    );
}

#[test]
fn self_search_finds_node_type_enum() {
    let engine = HermesEngine::in_memory("self-search-enum").unwrap();
    let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
    let pipeline = IngestionPipeline::new(&graph);

    let dir = tempfile::TempDir::new().unwrap();
    copy_source_to(dir.path());
    pipeline.ingest_directory(dir.path()).unwrap();

    let search = SearchEngine::new(&graph, engine.search_cache());
    let response = search.search("NodeType", 10, &SearchMode::Smart).unwrap();

    assert!(!response.pointers.is_empty(), "must find NodeType enum");
}

#[test]
fn self_fetch_returns_content_for_found_node() {
    let engine = HermesEngine::in_memory("self-fetch").unwrap();
    let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
    let pipeline = IngestionPipeline::new(&graph);

    let dir = tempfile::TempDir::new().unwrap();
    copy_source_to(dir.path());
    pipeline.ingest_directory(dir.path()).unwrap();

    let search = SearchEngine::new(&graph, engine.search_cache());
    let response = search.search("HermesEngine", 5, &SearchMode::Smart).unwrap();

    assert!(!response.pointers.is_empty(), "must find HermesEngine");

    let node_id = &response.pointers[0].id;
    let fetch_result: Option<FetchResponse> = search.fetch(node_id).unwrap();
    assert!(fetch_result.is_some(), "must fetch content for node");
    let fetched = fetch_result.unwrap();
    assert!(!fetched.content.is_empty(), "fetched content must not be empty");
    assert!(
        fetched.file_path.contains("lib.rs") || fetched.file_path.contains(".rs"),
        "file path should reference a source file"
    );
    assert!(fetched.token_count > 0, "token count must be positive");
}

#[test]
fn self_search_cache_hits_on_repeat_query() {
    let engine = HermesEngine::in_memory("self-cache").unwrap();
    let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
    let pipeline = IngestionPipeline::new(&graph);

    let dir = tempfile::TempDir::new().unwrap();
    copy_source_to(dir.path());
    pipeline.ingest_directory(dir.path()).unwrap();

    let search = SearchEngine::new(&graph, engine.search_cache());

    let r1 = search.search("WeightStore", 5, &SearchMode::Smart).unwrap();
    let r2 = search.search("WeightStore", 5, &SearchMode::Smart).unwrap();

    assert_eq!(r1.pointers.len(), r2.pointers.len());
}

#[test]
fn self_accounting_tracks_queries() {
    let engine = HermesEngine::in_memory("self-acct").unwrap();
    let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
    let pipeline = IngestionPipeline::new(&graph);

    let dir = tempfile::TempDir::new().unwrap();
    copy_source_to(dir.path());
    pipeline.ingest_directory(dir.path()).unwrap();

    let acct = Accountant::new(
        engine.db().clone(),
        engine.project_id(),
        engine.session_id(),
    );

    let initial = acct.get_session_stats().unwrap();
    assert_eq!(initial.total_queries, 0);

    acct.record_query("self_test_query", 100, 50, 2000)
        .unwrap();
    acct.record_query("another_query", 30, 10, 500).unwrap();

    let stats = acct.get_session_stats().unwrap();
    assert_eq!(stats.total_queries, 2);
    assert_eq!(stats.total_pointer_tokens, 130);
    assert_eq!(stats.total_fetched_tokens, 60);
    assert!(stats.cumulative_savings_pct > 0.0);

    let recent = acct.get_stats_since(Some(Duration::from_secs(3600))).unwrap();
    assert_eq!(recent.total_queries, 2);
}

#[test]
fn self_temporal_add_and_list_facts() {
    let engine = HermesEngine::in_memory("self-temporal").unwrap();
    let store = TemporalStore::new(engine.db().clone(), engine.project_id());

    let id = store
        .add_fact(AddFactInput {
            fact_type: FactType::Architecture,
            content: "Hermes uses a graph-weighted BM25 search pipeline",
            topic: Some("search-architecture"),
            tags: vec!["search".into(), "bm25".into()],
            confidence: Some(0.95),
            ..Default::default()
        })
        .unwrap();
    assert!(!id.is_empty(), "fact id must be non-empty");

    let facts = store.get_active_facts(&FactFilter::default()).unwrap();
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].content, "Hermes uses a graph-weighted BM25 search pipeline");
    assert_eq!(facts[0].fact_type, FactType::Architecture);
    assert_eq!(facts[0].tags, vec!["search", "bm25"]);
    assert!((facts[0].confidence.unwrap() - 0.95).abs() < 1e-6);

    let filtered = store
        .get_active_facts(&FactFilter {
            fact_type: Some(FactType::Architecture),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(filtered.len(), 1);

    let no_match = store
        .get_active_facts(&FactFilter {
            fact_type: Some(FactType::Learning),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(no_match.len(), 0);
}

#[test]
fn self_temporal_expire_fact() {
    let engine = HermesEngine::in_memory("self-temporal-expire").unwrap();
    let store = TemporalStore::new(engine.db().clone(), engine.project_id());

    let id = store
        .add_fact(AddFactInput {
            fact_type: FactType::Decision,
            content: "Use rusqlite with bundled SQLite",
            ..Default::default()
        })
        .unwrap();

    let before = store.get_active_facts(&FactFilter::default()).unwrap();
    assert_eq!(before.len(), 1);

    store.expire_fact(&id, None).unwrap();

    let after = store.get_active_facts(&FactFilter::default()).unwrap();
    assert_eq!(after.len(), 0);

    let with_expired = store
        .get_active_facts(&FactFilter {
            include_expired: true,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(with_expired.len(), 1);
    assert!(with_expired[0].valid_to.is_some());
}

#[test]
fn self_weight_store_read_write() {
    let engine = HermesEngine::in_memory("self-weight").unwrap();
    let store = WeightStore::new(engine.db().clone());

    let default = store.get_weight("some_node").unwrap();
    assert!((default - 1.0).abs() < f64::EPSILON);

    store.adjust_weight("node_a", 0.15).unwrap();
    let w = store.get_weight("node_a").unwrap();
    assert!(w > 1.0, "reinforced weight must be > 1.0, got {w}");

    store.adjust_weight("node_a", -0.10).unwrap();
    let d = store.get_weight("node_a").unwrap();
    assert!(d < w, "decayed weight must be less than {w}, got {d}");

    let non_default = store.list_non_default().unwrap();
    assert!(!non_default.is_empty(), "must have a non-default weight");

    let rec = store.get_record("node_a").unwrap();
    assert!(rec.is_some(), "node_a must have a weight record");
    let w = rec.unwrap().weight;
    assert!(
        (w - 1.05).abs() < 1e-9,
        "expected weight ~1.05, got {w}"
    );
}

#[test]
fn self_search_weight_interaction() {
    let engine = HermesEngine::in_memory("self-search-weight").unwrap();
    let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
    let pipeline = IngestionPipeline::new(&graph);
    let weight = WeightStore::new(engine.db().clone());

    let dir = tempfile::TempDir::new().unwrap();
    copy_source_to(dir.path());
    pipeline.ingest_directory(dir.path()).unwrap();

    let search = SearchEngine::new(&graph, engine.search_cache());
    let r_normal = search.search("TemporalStore", 10, &SearchMode::Smart).unwrap();
    assert!(!r_normal.pointers.is_empty());

    for p in &r_normal.pointers {
        weight.adjust_weight(&p.id, 0.2).unwrap();
    }
}
