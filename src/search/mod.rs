// ChartApp/hermes-engine/src/search/mod.rs
pub mod fts;
pub mod literal;
pub(crate) mod search_support;
pub mod vector;

use crate::graph::KnowledgeGraph;
use crate::graph_types::{Node, NodeType};
use crate::pointer::{FetchResponse, PointerResponse};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub use search_support::estimate_tokens;

/// Short-circuit thresholds for tier skipping.
///
/// Spec: Skip L1+L2 only when L0 yields an exact match (score = 1.0) on
/// highly structural nodes (Struct, Trait, Class).  For prefix matches
/// (score >= 0.9), do NOT skip L1 — instead cap FTS5 candidate limits.
const SHORT_CIRCUIT_SKIP_ALL: f64 = 1.0; // Exact match only — skip L1 + L2
const PREFIX_MATCH_CAP: f64 = 0.9;       // Prefix match — cap L1 processing, do NOT skip

/// Structural node types that trigger full short-circuit on exact match.
fn is_structural_node_type(node_type: &NodeType) -> bool {
    matches!(node_type, NodeType::Struct | NodeType::Trait | NodeType::Interface)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SearchMode {
    Pointer,
    Smart,
    Full,
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub node: Node,
    pub score: f64,
    pub tier: SearchTier,
    pub matched_content: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SearchTier {
    L0Literal,
    L1Fts,
    L2Vector,
}

pub struct SearchEngine<'a> {
    pub(crate) graph: &'a KnowledgeGraph,
    /// Shared search result cache (lives on HermesEngine).
    pub(crate) search_cache: Arc<Mutex<crate::SearchCacheMap>>,
    /// Per-engine fetch content cache (keyed on file_path + line range).
    pub(crate) fetch_cache: Mutex<HashMap<(String, i64, i64), String>>,
}

impl<'a> SearchEngine<'a> {
    pub fn new(graph: &'a KnowledgeGraph, search_cache: Arc<Mutex<crate::SearchCacheMap>>) -> Self {
        Self {
            graph,
            search_cache,
            fetch_cache: Mutex::new(HashMap::new()),
        }
    }

    pub fn search(&self, query: &str, top_k: usize, mode: &SearchMode) -> Result<PointerResponse> {
        // Check search cache first
        let cache_key = format!("{}:{}", query.trim().to_lowercase(), top_k);
        if let Some(cached) = search_support::get_from_cache(&self.search_cache, &cache_key) {
            return Ok(cached);
        }

        // L0: literal search (SQL-indexed, no full table scan)
        let l0_results = literal::literal_search(self.graph, query)?;

        // Short-circuit: only skip L1+L2 for exact score=1.0 matches on structural nodes
        if l0_results.iter().any(|r| {
            (r.score - SHORT_CIRCUIT_SKIP_ALL).abs() < f64::EPSILON
                && is_structural_node_type(&r.node.node_type)
        }) {
            let weights = search_support::load_weights(self.graph, &l0_results);
            let merged = search_support::deduplicate_and_rank(l0_results, top_k, &weights);
            let pointers = search_support::results_to_pointers(&merged, mode);
            let response = PointerResponse::build(pointers, 0);
            search_support::insert_into_cache(&self.search_cache, cache_key, response.clone());
            return Ok(response);
        }

        // Prefix match (score >= 0.9): run L1 with reduced limit, do NOT skip
        let prefix_match = l0_results.iter().any(|r| r.score >= PREFIX_MATCH_CAP);

        let mut all_results: Vec<SearchResult> = Vec::new();
        all_results.extend(l0_results);

        // L1: FTS search with reduced limit for prefix matches
        let fts_limit = if prefix_match { 5 } else { 20 };
        let l1_results = fts::fts_search_with_limit(self.graph, query, fts_limit)?;
        all_results.extend(l1_results);

        // Build deduplicated candidate set for graph-weight enhancement
        let mut seen = std::collections::HashSet::new();
        let l0_l1_node_ids: Vec<String> = all_results
            .iter()
            .filter(|r| seen.insert(r.node.id.clone()))
            .map(|r| r.node.id.clone())
            .collect();

        // Apply graph-weighted BM25 enhancement to L0+L1 scores
        let _ = vector::apply_graph_weight(self.graph, &l0_l1_node_ids, &mut all_results);

        let weights = search_support::load_weights(self.graph, &all_results);
        let merged = search_support::deduplicate_and_rank(all_results, top_k, &weights);
        let pointers = search_support::results_to_pointers(&merged, mode);
        let response = PointerResponse::build(pointers, 0);
        search_support::insert_into_cache(&self.search_cache, cache_key, response.clone());
        Ok(response)
    }

    pub fn fetch(&self, pointer_id: &str) -> Result<Option<FetchResponse>> {
        let node = self.graph.get_node(pointer_id)?;
        let Some(node) = node else {
            return Ok(None);
        };

        // Fetch content cache
        let content = search_support::read_node_content_cached(&self.fetch_cache, &node)?;
        let stale = search_support::detect_staleness(&node, &content);

        // Word-count based token estimate
        let token_count = estimate_tokens(&content);

        Ok(Some(FetchResponse {
            pointer_id: node.id.clone(),
            content,
            file_path: node.file_path.unwrap_or_default(),
            start_line: node.start_line.unwrap_or(0),
            end_line: node.end_line.unwrap_or(0),
            token_count,
            content_tokens: node.content_tokens,
            is_stale: stale.is_stale,
            stale_reason: stale.reason,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_circuit_thresholds_are_correct() {
        assert_eq!(SHORT_CIRCUIT_SKIP_ALL, 1.0);
        assert!(PREFIX_MATCH_CAP < SHORT_CIRCUIT_SKIP_ALL);
        assert!(PREFIX_MATCH_CAP > 0.0);
    }

    #[test]
    fn structural_node_types_includes_struct_trait_interface() {
        assert!(is_structural_node_type(&NodeType::Struct));
        assert!(is_structural_node_type(&NodeType::Trait));
        assert!(is_structural_node_type(&NodeType::Interface));
        assert!(!is_structural_node_type(&NodeType::Function));
        assert!(!is_structural_node_type(&NodeType::File));
        assert!(!is_structural_node_type(&NodeType::Enum));
    }
}
