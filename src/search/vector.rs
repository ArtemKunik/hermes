use crate::graph::KnowledgeGraph;
use crate::search::{SearchResult, SearchTier};
use crate::vector_ops::tokenize;
use anyhow::Result;
use std::collections::HashSet;

const VECTOR_LIMIT: usize = 20;
const MIN_SCORE: f64 = 0.20;

pub fn vector_search(
    graph: &KnowledgeGraph,
    query: &str,
    candidate_ids: Option<&HashSet<String>>,
) -> Result<Vec<SearchResult>> {
    // Use neural embeddings when available, local token-hash otherwise
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
    let mut results = node_vectors
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

    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.truncate(VECTOR_LIMIT);
    Ok(results)
}

fn cosine_similarity(lhs: &[f32], rhs: &[f32]) -> f64 {
    lhs.iter()
        .zip(rhs.iter())
        .map(|(a, b)| (*a as f64) * (*b as f64))
        .sum::<f64>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{KnowledgeGraph, Node, NodeType};
    use crate::vector_ops::build_vector;
    use crate::HermesEngine;

    #[test]
    fn cosine_similarity_is_high_for_similar_text() {
        let lhs = build_vector(&tokenize("fetch exchange rate currency"));
        let rhs = build_vector(&tokenize("exchange rate service currency"));
        let score = cosine_similarity(&lhs, &rhs);
        assert!(score > 0.4);
    }

    #[test]
    fn cosine_similarity_is_low_for_unrelated_text() {
        let lhs = build_vector(&tokenize("redis pubsub worker"));
        let rhs = build_vector(&tokenize("currency exchange rate"));
        let score = cosine_similarity(&lhs, &rhs);
        assert!(score < 0.4);
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
                })
                .unwrap();
        }

        let candidates = HashSet::from([String::from("n2")]);
        let results = vector_search(&graph, "vector engine", Some(&candidates)).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].node.id, "n2");
    }
}
