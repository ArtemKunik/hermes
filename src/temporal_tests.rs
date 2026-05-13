use super::*;
use crate::HermesEngine;

#[test]
fn add_and_retrieve_fact() {
    let engine = HermesEngine::in_memory("test").unwrap();
    let store = TemporalStore::new(engine.db().clone(), "test");

    let id = store
        .add_fact(
            None,
            FactType::Architecture,
            "Backend uses Axum + Tokio",
            Some("initial setup"),
        )
        .unwrap();

    let facts = store.get_active_facts(None).unwrap();
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].id, id);
    assert_eq!(facts[0].content, "Backend uses Axum + Tokio");
    assert!(facts[0].valid_to.is_none());
}

#[test]
fn invalidate_fact_sets_valid_to() {
    let engine = HermesEngine::in_memory("test").unwrap();
    let store = TemporalStore::new(engine.db().clone(), "test");

    let id = store
        .add_fact(None, FactType::Decision, "Use SQLite for storage", None)
        .unwrap();

    store.invalidate_fact(&id, None).unwrap();

    let active = store.get_active_facts(None).unwrap();
    assert!(active.is_empty());
}

#[test]
fn supersede_fact_creates_chain() {
    let engine = HermesEngine::in_memory("test").unwrap();
    let store = TemporalStore::new(engine.db().clone(), "test");

    let old_id = store
        .add_fact(None, FactType::Decision, "Use ChromaDB", None)
        .unwrap();

    let new_id = store
        .add_fact(None, FactType::Decision, "Use Qdrant instead", None)
        .unwrap();

    store.invalidate_fact(&old_id, Some(&new_id)).unwrap();

    let active = store.get_active_facts(None).unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].content, "Use Qdrant instead");
}

#[test]
fn filter_by_fact_type() {
    let engine = HermesEngine::in_memory("test").unwrap();
    let store = TemporalStore::new(engine.db().clone(), "test");

    store
        .add_fact(None, FactType::Architecture, "Axum backend", None)
        .unwrap();
    store
        .add_fact(None, FactType::Decision, "Use Rust", None)
        .unwrap();

    let arch_facts = store
        .get_active_facts(Some(&FactType::Architecture))
        .unwrap();
    assert_eq!(arch_facts.len(), 1);
    assert_eq!(arch_facts[0].content, "Axum backend");
}

#[test]
fn get_fact_history_returns_all_versions_for_node() {
    use crate::graph::{KnowledgeGraph, Node, NodeType};
    let engine = HermesEngine::in_memory("test-hist").unwrap();
    let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
    let real_node = Node {
        id: "node-1".to_string(),
        project_id: engine.project_id().to_string(),
        name: "some_fn".to_string(),
        node_type: NodeType::Function,
        file_path: None,
        start_line: None,
        end_line: None,
        summary: None,
        content_hash: None,
    };
    graph.add_node(&real_node).unwrap();

    let store = TemporalStore::new(engine.db().clone(), engine.project_id());
    let id1 = store
        .add_fact(Some("node-1"), FactType::Decision, "Old decision", None)
        .unwrap();
    let id2 = store
        .add_fact(Some("node-1"), FactType::Decision, "New decision", None)
        .unwrap();
    store.invalidate_fact(&id1, Some(&id2)).unwrap();

    let history = store.get_fact_history("node-1").unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].content, "New decision");
    assert_eq!(history[1].content, "Old decision");
    assert!(history[1].valid_to.is_some());
}

#[test]
fn get_fact_history_returns_empty_for_unknown_node() {
    let engine = HermesEngine::in_memory("test-hist2").unwrap();
    let store = TemporalStore::new(engine.db().clone(), "test-hist2");
    let history = store.get_fact_history("missing-node").unwrap();
    assert!(history.is_empty());
}

#[test]
fn fact_type_parse_str_unknown_falls_back_to_decision() {
    assert_eq!(FactType::parse_str("unknown_type"), FactType::Decision);
}

#[test]
fn fact_type_roundtrip_all_variants() {
    let variants = [
        FactType::Architecture,
        FactType::ApiContract,
        FactType::Decision,
        FactType::ErrorPattern,
        FactType::Constraint,
        FactType::Learning,
    ];
    for v in &variants {
        assert_eq!(&FactType::parse_str(v.as_str()), v);
    }
}

#[test]
fn invalidate_with_superseded_by_sets_chain() {
    let engine = HermesEngine::in_memory("test-chain").unwrap();
    let store = TemporalStore::new(engine.db().clone(), "test-chain");

    let old = store
        .add_fact(None, FactType::Constraint, "Max 100 connections", None)
        .unwrap();
    let new = store
        .add_fact(None, FactType::Constraint, "Max 500 connections", None)
        .unwrap();
    store.invalidate_fact(&old, Some(&new)).unwrap();

    let all_facts: Vec<_> = store
        .get_active_facts(None)
        .unwrap()
        .into_iter()
        .chain(vec![])
        .collect();
    assert_eq!(all_facts.len(), 1);
    assert_eq!(all_facts[0].content, "Max 500 connections");
}

#[test]
fn source_reference_is_stored_and_retrieved() {
    let engine = HermesEngine::in_memory("test-ref").unwrap();
    let store = TemporalStore::new(engine.db().clone(), "test-ref");
    store
        .add_fact(
            None,
            FactType::Learning,
            "Always validate input",
            Some("PR #42"),
        )
        .unwrap();
    let facts = store.get_active_facts(None).unwrap();
    assert_eq!(facts[0].source_reference.as_deref(), Some("PR #42"));
}
