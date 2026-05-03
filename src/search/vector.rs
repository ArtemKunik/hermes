use crate::graph::{KnowledgeGraph, Node};
use crate::search::{SearchResult, SearchTier};
use anyhow::Result;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

const VECTOR_DIMENSION: usize = 256;
const VECTOR_LIMIT: usize = 20;
const MIN_SCORE: f64 = 0.20;

/// Boost multiplier applied to source-code files so they rank above
/// natural-language documents (JSON data, markdown) for code queries.
const SOURCE_CODE_EXTENSIONS: &[&str] =
    &["rs", "ts", "tsx", "jsx", "js", "py", "kt", "css", "toml"];
const SOURCE_CODE_BOOST: f64 = 1.5;
/// Penalty multiplier for data/doc files that should rank below code.
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

/// Vector reranking over a candidate set.
///
/// If `candidates` is non-empty, only those nodes are scored — O(candidates).
/// If `candidates` is empty, falls back to a full index scan — O(N).
/// Callers should pass L0+L1 result nodes as candidates whenever available.
pub fn vector_search(
    graph: &KnowledgeGraph,
    query: &str,
    candidates: Vec<Node>,
) -> Result<Vec<SearchResult>> {
    let query_tokens = tokenize(query);
    if query_tokens.is_empty() {
        return Ok(Vec::new());
    }

    let nodes = if candidates.is_empty() {
        graph.get_all_nodes()?
    } else {
        candidates
    };

    let query_vec = build_vector(&query_tokens);
    let mut results = nodes
        .into_iter()
        .filter_map(|node| {
            let text = combined_node_text(&node);
            let tokens = tokenize(&text);
            if tokens.is_empty() {
                return None;
            }

            let node_vec = build_vector(&tokens);
            let raw_score = cosine_similarity(&query_vec, &node_vec);
            let score = raw_score * file_type_multiplier(node.file_path.as_ref());
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
}
