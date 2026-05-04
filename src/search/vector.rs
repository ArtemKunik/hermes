use crate::graph::KnowledgeGraph;
use crate::search::{SearchResult, SearchTier};
use crate::vector_ops::{build_vector, tokenize};
use anyhow::Result;

const VECTOR_LIMIT: usize = 20;
const MIN_SCORE: f64 = 0.20;

pub fn vector_search(graph: &KnowledgeGraph, query: &str) -> Result<Vec<SearchResult>> {
    let query_tokens = tokenize(query);
    if query_tokens.is_empty() {
        return Ok(Vec::new());
    }

    let query_vec = build_vector(&query_tokens);
    let mut results = graph
        .get_all_node_vectors()?
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
    use crate::vector_ops::build_vector;

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
}
