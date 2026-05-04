// ChartApp/hermes-engine/src/search/fts.rs
use crate::graph::{KnowledgeGraph, Node};
use crate::search::{SearchResult, SearchTier};
use anyhow::Result;

const FTS_LIMIT: usize = 20;
const STRATEGY_MIN_RESULTS: usize = 3;
const MAX_QUERY_WORDS: usize = 10;

/// Task 2.1: Three-strategy FTS with phrase → AND-prefix → OR fallback.
/// Truncates to first 10 meaningful words to avoid degenerate queries on long strings.
pub fn fts_search(graph: &KnowledgeGraph, query: &str) -> Result<Vec<SearchResult>> {
    let words = extract_query_terms(query);

    if words.is_empty() {
        return Ok(Vec::new());
    }

    if words.len() == 1 {
        let single = quote_fts_term(&words[0]);
        return Ok(to_search_results(graph.fts_search(&single, FTS_LIMIT)?));
    }

    // Strategy 1: Exact phrase match — highest precision
    let phrase_query = format!("\"{}\"", words.join(" "));
    let s1 = graph.fts_search(&phrase_query, FTS_LIMIT)?;
    if s1.len() >= STRATEGY_MIN_RESULTS {
        return Ok(to_search_results(s1));
    }

    // Strategy 2: AND-prefix match — good recall for multi-token queries
    let and_query = words
        .iter()
        .map(|w| format!("{}*", quote_fts_term(w)))
        .collect::<Vec<_>>()
        .join(" AND ");
    let s2 = graph.fts_search(&and_query, FTS_LIMIT)?;
    if s2.len() >= STRATEGY_MIN_RESULTS {
        return Ok(to_search_results(s2));
    }

    // Strategy 3: OR fallback — maximum recall
    let or_query = words
        .iter()
        .map(|w| quote_fts_term(w))
        .collect::<Vec<_>>()
        .join(" OR ");
    Ok(to_search_results(graph.fts_search(&or_query, FTS_LIMIT)?))
}

fn extract_query_terms(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .filter(|w| !is_fts_operator(w))
        .map(sanitize_term)
        .filter(|w| !w.is_empty())
        .take(MAX_QUERY_WORDS)
        .collect()
}

fn sanitize_term(word: &str) -> String {
    word
        .trim_matches('"')
        .replace('"', "")
        .trim()
        .to_string()
}

fn quote_fts_term(term: &str) -> String {
    format!("\"{}\"", term)
}

fn to_search_results(raw: Vec<(Node, f64)>) -> Vec<SearchResult> {
    raw.into_iter()
        .map(|(node, rank)| SearchResult {
            node,
            score: normalize_bm25_score(rank),
            tier: SearchTier::L1Fts,
            matched_content: None,
        })
        .collect()
}

fn is_fts_operator(word: &str) -> bool {
    matches!(word.to_uppercase().as_str(), "AND" | "OR" | "NOT" | "NEAR")
}

fn normalize_bm25_score(rank: f64) -> f64 {
    let abs_rank = rank.abs();
    if abs_rank < 0.001 {
        return 0.5;
    }
    (1.0 - 1.0 / (1.0 + abs_rank)).min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HermesEngine;

    fn make_graph(engine: &HermesEngine) -> crate::graph::KnowledgeGraph {
        crate::graph::KnowledgeGraph::new(engine.db().clone(), engine.project_id())
    }

    #[test]
    fn single_word_uses_phrase_strategy() {
        let sanitized = prepare_test_query("main");
        assert_eq!(sanitized, "\"main\"");
    }

    fn prepare_test_query(query: &str) -> String {
        let words = extract_query_terms(query);
        if words.len() == 1 {
            quote_fts_term(&words[0])
        } else {
            format!("\"{}\"", words.join(" "))
        }
    }

    #[test]
    fn filters_fts_operators() {
        let words = extract_query_terms("NOT main AND test OR foo");
        assert!(!words.iter().any(|w| w == "NOT"));
        assert!(!words.iter().any(|w| w == "AND"));
        assert!(!words.iter().any(|w| w == "OR"));
        assert!(words.iter().any(|w| w == "main"));
        assert!(words.iter().any(|w| w == "test"));
        assert!(words.iter().any(|w| w == "foo"));
    }

    #[test]
    fn truncates_to_ten_words() {
        let long_query = "a b c d e f g h i j k l m n";
        let words = extract_query_terms(long_query);
        assert_eq!(words.len(), MAX_QUERY_WORDS);
    }

    #[test]
    fn strips_user_quotes_from_path_terms() {
        let words = extract_query_terms("\"/api/alerts\" handler");
        assert_eq!(words, vec!["/api/alerts".to_string(), "handler".to_string()]);
    }

    #[test]
    fn single_quoted_path_produces_valid_fts_term() {
        let query = prepare_test_query("\"/api/alerts\"");
        assert_eq!(query, "\"/api/alerts\"");
    }

    #[test]
    fn bm25_normalization() {
        assert!(normalize_bm25_score(-5.0) > 0.5);
        assert!(normalize_bm25_score(-10.0) > normalize_bm25_score(-5.0));
        assert!(normalize_bm25_score(0.0) < 0.6);
    }

    #[test]
    fn empty_query_returns_empty() {
        let engine = HermesEngine::in_memory("test-fts").unwrap();
        let graph = make_graph(&engine);
        let results = fts_search(&graph, "").unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn operator_only_query_returns_empty() {
        let engine = HermesEngine::in_memory("test-fts").unwrap();
        let graph = make_graph(&engine);
        let results = fts_search(&graph, "AND OR NOT").unwrap();
        assert!(results.is_empty());
    }
}

