use crate::graph::KnowledgeGraph;
use crate::graph_ops::split_identifier;
use crate::graph_types::Node;
use crate::search::{SearchResult, SearchTier};
use anyhow::Result;

const FTS_LIMIT: usize = 20;
const STRATEGY_MIN_RESULTS: usize = 3;
const MAX_QUERY_WORDS: usize = 10;

pub fn fts_search_with_limit(graph: &KnowledgeGraph, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
    let words = extract_query_terms(query);

    if words.is_empty() {
        return Ok(Vec::new());
    }

    if words.len() == 1 {
        let single = quote_fts_term(&words[0]);
        return Ok(to_search_results(graph.fts_search(&single, limit)?));
    }

    let phrase_query = format!("\"{}\"", words.join(" "));
    let s1 = graph.fts_search(&phrase_query, limit)?;
    if s1.len() >= STRATEGY_MIN_RESULTS {
        return Ok(to_search_results(s1));
    }

    let and_query = words
        .iter()
        .map(|w| format!("{}*", quote_fts_term(w)))
        .collect::<Vec<_>>()
        .join(" AND ");
    let s2 = graph.fts_search(&and_query, limit)?;
    if s2.len() >= STRATEGY_MIN_RESULTS {
        return Ok(to_search_results(s2));
    }

    let or_query = words
        .iter()
        .map(|w| quote_fts_term(w))
        .collect::<Vec<_>>()
        .join(" OR ");
    Ok(to_search_results(graph.fts_search(&or_query, limit)?))
}

pub fn fts_search(graph: &KnowledgeGraph, query: &str) -> Result<Vec<SearchResult>> {
    fts_search_with_limit(graph, query, FTS_LIMIT)
}

fn extract_query_terms(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .filter(|w| !is_fts_operator(w))
        .map(sanitize_term)
        .filter(|w| !w.is_empty())
        .flat_map(|w| {
            let splits: Vec<String> = split_identifier(&w);
            if splits.len() > 1 { splits } else { vec![w] }
        })
        .take(MAX_QUERY_WORDS)
        .collect()
}

fn sanitize_term(word: &str) -> String {
    word.trim_matches('"').replace('"', "").trim().to_string()
}

fn quote_fts_term(term: &str) -> String {
    format!("\"{}\"", term)
}

fn is_fts_operator(word: &str) -> bool {
    matches!(word.to_uppercase().as_str(), "AND" | "OR" | "NOT")
}

fn to_search_results(raw: Vec<(Node, f64)>) -> Vec<SearchResult> {
    raw.into_iter()
        .map(|(node, score)| SearchResult {
            node,
            score,
            tier: SearchTier::L1Fts,
            matched_content: None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn empty_query_returns_empty_results() {
        let engine = crate::HermesEngine::in_memory("test-fts-empty").unwrap();
        let graph = crate::graph::KnowledgeGraph::new(engine.db().clone(), engine.project_id());
        let results = fts_search(&graph, "").unwrap();
        assert!(results.is_empty());
    }
}
