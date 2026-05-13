use super::*;
use crate::graph::{KnowledgeGraph, Node, NodeType};
use crate::HermesEngine;

#[test]
fn dedup_keeps_highest_score() {
    let node = Node {
        id: "n1".to_string(),
        project_id: "test".to_string(),
        name: "test_fn".to_string(),
        node_type: NodeType::Function,
        file_path: None,
        start_line: None,
        end_line: None,
        summary: None,
        content_hash: None,
    };

    let results = vec![
        SearchResult {
            node: node.clone(),
            score: 0.5,
            tier: SearchTier::L1Fts,
            matched_content: None,
        },
        SearchResult {
            node: node.clone(),
            score: 0.9,
            tier: SearchTier::L0Literal,
            matched_content: None,
        },
    ];

    let deduped = SearchEngine::deduplicate_and_rank(results, 10);
    assert_eq!(deduped.len(), 1);
    assert_eq!(deduped[0].tier, SearchTier::L0Literal);
}

#[test]
fn short_circuit_skips_on_high_l0_confidence() {
    assert!(SHORT_CIRCUIT_SKIP_ALL > SHORT_CIRCUIT_SKIP_L2);
    assert!(SHORT_CIRCUIT_SKIP_ALL <= 1.0);
    assert!(SHORT_CIRCUIT_SKIP_L2 > 0.0);
}

#[test]
fn cache_miss_then_hit() {
    let engine = crate::HermesEngine::in_memory("test-cache-mod").unwrap();
    let cache = engine.search_cache();
    let dummy = PointerResponse::build(vec![], 0);
    {
        let mut c = cache.lock().unwrap();
        c.insert("key:10".to_string(), (dummy, Instant::now()));
    }
    let c = cache.lock().unwrap();
    assert!(c.contains_key("key:10"));
}

#[test]
fn estimate_tokens_word_count_based() {
    let tokens = estimate_tokens("hello world foo bar");
    assert_eq!(tokens, 6);
}

#[test]
fn estimate_tokens_empty() {
    assert_eq!(estimate_tokens(""), 0);
}

#[test]
fn search_pipeline_returns_pointers_for_indexed_data() {
    let engine = HermesEngine::in_memory("test-search-pipeline").unwrap();
    let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());

    let node = Node {
        id: "search-node-1".to_string(),
        project_id: engine.project_id().to_string(),
        name: "fetch_user_data".to_string(),
        node_type: NodeType::Function,
        file_path: None,
        start_line: Some(1),
        end_line: Some(10),
        summary: Some("Fetches user data from API".to_string()),
        content_hash: None,
    };
    graph.add_node(&node).unwrap();
    graph.index_fts(&node, "fetches user data from the remote API").unwrap();

    let search = SearchEngine::new(&graph, engine.search_cache());
    let response = search.search("fetch_user", 5, &SearchMode::Smart).unwrap();
    assert!(!response.pointers.is_empty());
}

#[test]
fn fetch_returns_content_for_valid_node() {
    let engine = HermesEngine::in_memory("test-fetch").unwrap();
    let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());

    let node = Node {
        id: "fetch-node-1".to_string(),
        project_id: engine.project_id().to_string(),
        name: "get_config".to_string(),
        node_type: NodeType::Function,
        file_path: None,
        start_line: Some(1),
        end_line: Some(5),
        summary: Some("Returns config".to_string()),
        content_hash: None,
    };
    graph.add_node(&node).unwrap();

    let search = SearchEngine::new(&graph, engine.search_cache());
    let result = search.fetch("fetch-node-1").unwrap();
    assert!(result.is_some());
    assert_eq!(result.unwrap().pointer_id, "fetch-node-1");
}

#[test]
fn fetch_returns_none_for_missing_node() {
    let engine = HermesEngine::in_memory("test-fetch-missing").unwrap();
    let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
    let search = SearchEngine::new(&graph, engine.search_cache());
    let result = search.fetch("nonexistent-id").unwrap();
    assert!(result.is_none());
}

#[test]
fn collect_candidate_ids_extracts_ids() {
    let node1 = Node {
        id: "c1".to_string(),
        project_id: "test".to_string(),
        name: "fn1".to_string(),
        node_type: NodeType::Function,
        file_path: None,
        start_line: None,
        end_line: None,
        summary: None,
        content_hash: None,
    };
    let node2 = Node {
        id: "c2".to_string(),
        project_id: "test".to_string(),
        name: "fn2".to_string(),
        node_type: NodeType::Function,
        file_path: None,
        start_line: None,
        end_line: None,
        summary: None,
        content_hash: None,
    };

    let results = vec![
        SearchResult { node: node1, score: 1.0, tier: SearchTier::L0Literal, matched_content: None },
        SearchResult { node: node2, score: 0.5, tier: SearchTier::L1Fts, matched_content: None },
    ];

    let ids = SearchEngine::collect_candidate_ids(&results);
    assert_eq!(ids.len(), 2);
    assert!(ids.contains("c1"));
    assert!(ids.contains("c2"));
}

#[test]
fn results_to_pointers_conversion() {
    let node = Node {
        id: "p1".to_string(),
        project_id: "test".to_string(),
        name: "my_handler".to_string(),
        node_type: NodeType::Function,
        file_path: Some("src/handler.rs".to_string()),
        start_line: Some(10),
        end_line: Some(25),
        summary: Some("Handles requests".to_string()),
        content_hash: None,
    };

    let results = vec![SearchResult {
        node,
        score: 0.95,
        tier: SearchTier::L0Literal,
        matched_content: None,
    }];

    let pointers = SearchEngine::results_to_pointers(&results, &SearchMode::Smart);
    assert_eq!(pointers.len(), 1);
    assert_eq!(pointers[0].id, "p1");
    assert_eq!(pointers[0].source, "src/handler.rs");
    assert_eq!(pointers[0].chunk, "my_handler");
    assert_eq!(pointers[0].relevance, 0.95);
}

#[test]
fn fetch_reads_file_content() {
    let dir = tempfile::TempDir::new().unwrap();
    let file_path = dir.path().join("test.rs");
    std::fs::write(&file_path, "line1\nline2\nline3\nline4\n").unwrap();
    let file_path_str = file_path.to_str().unwrap().to_string();

    let engine = HermesEngine::in_memory("test-fetch-file").unwrap();
    let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
    let node = Node {
        id: "f1".to_string(),
        project_id: engine.project_id().to_string(),
        name: "test_fn".to_string(),
        node_type: NodeType::Function,
        file_path: Some(file_path_str),
        start_line: Some(2),
        end_line: Some(3),
        summary: None,
        content_hash: None,
    };
    graph.add_node(&node).unwrap();

    let search = SearchEngine::new(&graph, engine.search_cache());
    let result = search.fetch("f1").unwrap().unwrap();
    assert_eq!(result.content, "line2\nline3");
}

#[test]
fn fetch_missing_file_returns_error_text() {
    let engine = HermesEngine::in_memory("test-fetch-missing-file").unwrap();
    let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
    let node = Node {
        id: "f2".to_string(),
        project_id: engine.project_id().to_string(),
        name: "ghost_fn".to_string(),
        node_type: NodeType::Function,
        file_path: Some("/nonexistent/path/ghost.rs".to_string()),
        start_line: Some(1),
        end_line: Some(10),
        summary: None,
        content_hash: None,
    };
    graph.add_node(&node).unwrap();

    let search = SearchEngine::new(&graph, engine.search_cache());
    let result = search.fetch("f2").unwrap().unwrap();
    assert!(result.content.starts_with("[File not found:"));
}
