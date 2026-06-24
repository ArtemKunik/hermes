use crate::graph::KnowledgeGraph;
use crate::search::{SearchResult, SearchTier};
use crate::vector_ops::tokenize;
use anyhow::Result;
use rusqlite::params;
use std::collections::{HashMap, HashSet};

const VECTOR_LIMIT: usize = 20;
const MIN_SCORE: f64 = 0.20;

const GRAPH_BOOST_FACTOR: f64 = 0.15;
const MAX_GRAPH_BOOST: f64 = 2.5;

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

pub fn vector_search(
    graph: &KnowledgeGraph,
    query: &str,
    candidate_ids: Option<&HashSet<String>>,
) -> Result<Vec<SearchResult>> {
    let query_vec = crate::neural_embed::embed(query);
    if query_vec.is_empty() {
        let tokens = tokenize(query);
        if tokens.is_empty() {
            return Ok(Vec::new());
        }
    }
    if query_vec.is_empty() {
        return Ok(Vec::new());
    }
    let node_vectors = match candidate_ids {
        Some(ids) if !ids.is_empty() => {
            let mut ordered_ids: Vec<String> = ids.iter().cloned().collect();
            ordered_ids.sort();
            graph.get_node_vectors_by_ids(&ordered_ids)?
        }
        _ => graph.get_all_node_vectors()?,
    };
    let results = node_vectors
        .into_iter()
        .filter_map(|(node, node_vec)| {
            let score = cosine_similarity(&query_vec, &node_vec);
            if score < MIN_SCORE {
                return None;
            }
            Some(SearchResult {
                node,
                score,
                tier: SearchTier::L2Vector,
                matched_content: None,
            })
        })
        .collect::<Vec<_>>();
    Ok(results)
}

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

fn cosine_similarity(lhs: &[f32], rhs: &[f32]) -> f64 {
    lhs.iter()
        .zip(rhs.iter())
        .map(|(a, b)| (*a as f64) * (*b as f64))
        .sum::<f64>()
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
    use crate::graph::{KnowledgeGraph, Node, NodeType};
    use crate::vector_ops::build_vector;
    use crate::HermesEngine;

    #[test]
    fn file_type_multiplier_boosts_source_code() {
        let path = Some("src/search/vector.rs".to_string());
        assert!(
            (file_type_multiplier(path.as_ref()) - SOURCE_CODE_BOOST).abs() < f64::EPSILON,
        );
    }

    #[test]
    fn file_type_multiplier_penalises_json() {
        let path = Some("data.json".to_string());
        assert!(
            (file_type_multiplier(path.as_ref()) - DATA_FILE_PENALTY).abs() < f64::EPSILON,
        );
    }

    #[test]
    fn file_type_multiplier_is_neutral_for_unknown_ext() {
        let path = Some("script.sh".to_string());
        assert!(
            (file_type_multiplier(path.as_ref()) - 1.0).abs() < f64::EPSILON,
        );
    }

    #[test]
    fn file_type_multiplier_is_neutral_for_no_path() {
        assert!(
            (file_type_multiplier(None) - 1.0).abs() < f64::EPSILON,
        );
    }

    #[test]
    fn empty_results_not_affected() {
        let engine = crate::HermesEngine::in_memory("test-graph-weight").unwrap();
        let graph = crate::graph::KnowledgeGraph::new(engine.db().clone(), engine.project_id());
        let mut results: Vec<SearchResult> = Vec::new();
        assert!(apply_graph_weight(&graph, &[], &mut results).is_ok());
    }

    #[test]
    fn vector_search_can_be_limited_to_candidates() {
        let engine = HermesEngine::in_memory("vector-candidates").unwrap();
        let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
        for (id, name) in [
            ("n1", "search_index"),
            ("n2", "vector engine"),
            ("n3", "totally_unrelated"),
        ] {
            graph
                .add_node(&Node {
                    id: id.to_string(),
                    project_id: engine.project_id().to_string(),
                    name: name.to_string(),
                    node_type: NodeType::Function,
                    file_path: Some(format!("src/{id}.rs")),
                    start_line: Some(1),
                    end_line: Some(5),
                    summary: None,
                    content_hash: None,
                    content_tokens: None,
                    object_type: None,
                })
                .unwrap();
        }

        let candidates = HashSet::from([String::from("n2")]);
        let results = vector_search(&graph, "vector engine", Some(&candidates)).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].node.id, "n2");
    }
}
