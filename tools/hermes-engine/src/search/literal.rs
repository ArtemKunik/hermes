// ChartApp/hermes-engine/src/search/literal.rs
use crate::graph::KnowledgeGraph;
use crate::search::{SearchResult, SearchTier};
use anyhow::Result;

/// Task 1.1: Uses SQL index (LOWER(name) LIKE ?) instead of full table scan.
/// get_all_nodes() is never called from this function.
pub fn literal_search(graph: &KnowledgeGraph, query: &str) -> Result<Vec<SearchResult>> {
    let query_lower = query.to_lowercase();
    let nodes = graph.literal_search_by_name(query)?;

    let mut results: Vec<SearchResult> = nodes
        .into_iter()
        .map(|node| {
            let name_lower = node.name.to_lowercase();
            let score = compute_literal_score(&query_lower, &name_lower);
            SearchResult {
                node,
                score,
                tier: SearchTier::L0Literal,
                matched_content: None,
            }
        })
        .collect();

    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.truncate(20);
    Ok(results)
}

fn compute_literal_score(query: &str, name: &str) -> f64 {
    if name == query {
        return 1.0;
    }
    if name.starts_with(query) || name.ends_with(query) {
        return 0.9;
    }
    let query_len = query.len() as f64;
    let name_len = name.len().max(1) as f64;
    0.5 + (query_len / name_len) * 0.4
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match_scores_highest() {
        assert_eq!(compute_literal_score("main", "main"), 1.0);
    }

    #[test]
    fn prefix_match_scores_high() {
        let score = compute_literal_score("fetch", "fetch_exchange_rate");
        assert!(score >= 0.8);
    }

    #[test]
    fn partial_match_scores_moderate() {
        let score = compute_literal_score("rate", "exchange_rate_service");
        assert!(score > 0.5 && score < 0.9);
    }
}
