use crate::graph::KnowledgeGraph;
use crate::search::{SearchResult, SearchTier};
use anyhow::Result;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

const VECTOR_DIMENSION: usize = 256;
const VECTOR_LIMIT: usize = 20;
const MIN_SCORE: f64 = 0.20;

pub fn vector_search(graph: &KnowledgeGraph, query: &str) -> Result<Vec<SearchResult>> {
    let query_tokens = tokenize(query);
    if query_tokens.is_empty() {
        return Ok(Vec::new());
    }

    let query_vec = build_vector(&query_tokens);
    let mut results = graph
        .get_all_nodes()?
        .into_iter()
        .filter_map(|node| {
            let text = combined_node_text(&node);
            let tokens = tokenize(&text);
            if tokens.is_empty() {
                return None;
            }

            let node_vec = build_vector(&tokens);
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

fn combined_node_text(node: &crate::graph::Node) -> String {
    let mut text = String::new();
    text.push_str(&node.name);
    if let Some(summary) = &node.summary {
        text.push(' ');
        text.push_str(summary);
    }
    if let Some(path) = &node.file_path {
        text.push(' ');
        text.push_str(path);
    }
    text
}

fn tokenize(input: &str) -> Vec<String> {
    input
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .map(|part| part.trim().to_lowercase())
        .filter(|part| part.len() > 1)
        .collect()
}

fn build_vector(tokens: &[String]) -> Vec<f32> {
    let mut vec = vec![0.0f32; VECTOR_DIMENSION];
    for token in tokens {
        let index = stable_hash(token) % VECTOR_DIMENSION;
        vec[index] += 1.0;
    }
    normalize(&mut vec);
    vec
}

fn stable_hash(value: &str) -> usize {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish() as usize
}

fn normalize(vec: &mut [f32]) {
    let norm = vec
        .iter()
        .map(|v| (*v as f64) * (*v as f64))
        .sum::<f64>()
        .sqrt();
    if norm < f64::EPSILON {
        return;
    }
    for value in vec {
        *value /= norm as f32;
    }
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

    #[test]
    fn tokenize_ignores_short_tokens() {
        let tokens = tokenize("fn a fetch_exchange_rate");
        assert!(tokens.contains(&"fetch_exchange_rate".to_string()));
        assert!(!tokens.contains(&"a".to_string()));
    }

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
