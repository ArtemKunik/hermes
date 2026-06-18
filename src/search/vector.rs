use crate::graph::KnowledgeGraph;
use crate::search::SearchResult;
use anyhow::Result;
use rusqlite::params;
use std::collections::HashMap;

/// Boost factor applied per incoming edge to a node.
/// FTS score becomes: original_score * (1.0 + GRAPH_BOOST_FACTOR * log2(1 + incoming_edges))
const GRAPH_BOOST_FACTOR: f64 = 0.15;

/// Maximum graph boost multiplier (cap to avoid extreme outliers dominating).
const MAX_GRAPH_BOOST: f64 = 2.5;

/// Boost multiplier applied to source-code files so they rank above
/// natural-language documents (JSON data, markdown) for code queries.
const SOURCE_CODE_EXTENSIONS: &[&str] =
    &["rs", "ts", "tsx", "jsx", "js", "py", "kt", "css", "toml"];
const SOURCE_CODE_BOOST: f64 = 1.5;
const DATA_FILE_EXTENSIONS: &[&str] = &["json", "md"];
const DATA_FILE_PENALTY: f64 = 0.6;

fn file_type_multiplier(file_path: Option<&String>) -> f64 {
    let Some(path) = file_path else { return 1.0 };
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    if SOURCE_CODE_EXTENSIONS.contains(&ext) {
        return SOURCE_CODE_BOOST;
    }
    if DATA_FILE_EXTENSIONS.contains(&ext) {
        return DATA_FILE_PENALTY;
    }
    1.0
}

/// Apply graph-weighted BM25 enhancement to a set of search results.
///
/// Each result's score is boosted based on:
///   1. Incoming edge count (Calls, Uses, Imports) — nodes referenced by many
///      other code units rank higher.
///   2. File type — source code files are boosted, data/doc files penalized.
///
/// This replaces the previous hash-vector L2 rerank with zero-latency graph
/// topology signals, reallocating execution budget from embedding computation
/// to structural relevance scoring.
///
/// If `candidates` is non-empty, only those node IDs are queried for edge
/// counts — O(candidates).  Falls back to empty (no boost) when empty.
pub fn apply_graph_weight(
    graph: &KnowledgeGraph,
    candidates: &[String],
    results: &mut [SearchResult],
) -> Result<()> {
    if candidates.is_empty() || results.is_empty() {
        return Ok(());
    }

    let boosts = compute_graph_boosts(graph, candidates)?;

    for result in results.iter_mut() {
        let graph_boost = boosts.get(&result.node.id).copied().unwrap_or(1.0);
        let file_boost = file_type_multiplier(result.node.file_path.as_ref());
        result.score *= graph_boost * file_boost;
    }

    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(())
}

fn compute_graph_boosts(graph: &KnowledgeGraph, node_ids: &[String]) -> Result<HashMap<String, f64>> {
    graph.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT target_id, COUNT(*) as edge_count
             FROM edges
             WHERE target_id IN (SELECT id FROM nodes WHERE project_id = ?1)
               AND edge_type IN ('Calls', 'Uses', 'Imports')
               AND project_id = ?2
             GROUP BY target_id",
        )?;

        let rows = stmt.query_map(
            params![graph.project_id(), graph.project_id()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )?;

        let mut boosts = HashMap::new();
        for row in rows {
            let (target_id, count) = row?;
            let boost = 1.0 + GRAPH_BOOST_FACTOR * (1.0 + count as f64).log2();
            boosts.insert(target_id, boost.min(MAX_GRAPH_BOOST));
        }

        for id in node_ids {
            boosts.entry(id.clone()).or_insert(1.0);
        }

        Ok(boosts)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_type_multiplier_boosts_source_code() {
        let path = Some("src/search/vector.rs".to_string());
        assert!(
            (file_type_multiplier(path.as_ref()) - SOURCE_CODE_BOOST).abs() < f64::EPSILON,
            "Rust files should receive the source-code boost"
        );
    }

    #[test]
    fn file_type_multiplier_penalises_json() {
        let path = Some(".agent/research-pipeline/artifacts/synthesis.json".to_string());
        assert!(
            (file_type_multiplier(path.as_ref()) - DATA_FILE_PENALTY).abs() < f64::EPSILON,
            "JSON files should receive the data-file penalty"
        );
    }

    #[test]
    fn file_type_multiplier_is_neutral_for_unknown_ext() {
        let path = Some("script.sh".to_string());
        assert!(
            (file_type_multiplier(path.as_ref()) - 1.0).abs() < f64::EPSILON,
            "Unknown extensions should be neutral"
        );
    }

    #[test]
    fn file_type_multiplier_is_neutral_for_no_path() {
        assert!(
            (file_type_multiplier(None) - 1.0).abs() < f64::EPSILON,
            "Missing path should be neutral"
        );
    }

    #[test]
    fn empty_results_not_affected() {
        let engine = crate::HermesEngine::in_memory("test-graph-weight").unwrap();
        let graph = crate::graph::KnowledgeGraph::new(engine.db().clone(), engine.project_id());
        let mut results: Vec<SearchResult> = Vec::new();
        assert!(apply_graph_weight(&graph, &[], &mut results).is_ok());
    }
}
