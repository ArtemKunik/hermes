pub mod fts;
pub mod literal;
pub(crate) mod search_support;
pub mod vector;

use crate::graph::KnowledgeGraph;
use crate::graph_types::{Node, NodeType};
use crate::pointer::{FetchResponse, PointerResponse};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

pub use search_support::estimate_tokens;

const SHORT_CIRCUIT_SKIP_ALL: f64 = 1.0;
const PREFIX_MATCH_CAP: f64 = 0.9;

fn is_structural_node_type(node_type: &NodeType) -> bool {
    matches!(node_type, NodeType::Struct | NodeType::Trait | NodeType::Interface)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SearchMode {
    Smart,
    Precise,
    Fast,
}

impl Default for SearchMode {
    fn default() -> Self {
        Self::Smart
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub node: Node,
    pub score: f64,
    pub tier: SearchTier,
    pub matched_content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SearchTier {
    L0Literal,
    L1Fts,
    L2Vector,
}

pub struct SearchEngine<'a> {
    pub(crate) graph: &'a KnowledgeGraph,
    pub(crate) search_cache: Arc<Mutex<crate::SearchCacheMap>>,
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
        let cache_key = format!("{}:{}", query.trim().to_lowercase(), top_k);
        if let Some(cached) = search_support::get_from_cache(&self.search_cache, &cache_key) {
            return Ok(cached);
        }

        let l0_results = literal::literal_search(self.graph, query)?;

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

        let prefix_match = l0_results.iter().any(|r| r.score >= PREFIX_MATCH_CAP);

        let mut all_results: Vec<SearchResult> = Vec::new();
        all_results.extend(l0_results);

        let fts_limit = if prefix_match { 5 } else { 20 };
        let l1_results = fts::fts_search_with_limit(self.graph, query, fts_limit)?;
        all_results.extend(l1_results);

        let mut seen = std::collections::HashSet::new();
        let l0_l1_node_ids: Vec<String> = all_results
            .iter()
            .filter(|r| seen.insert(r.node.id.clone()))
            .map(|r| r.node.id.clone())
            .collect();

        let _ = vector::apply_graph_weight(self.graph, &l0_l1_node_ids, &mut all_results);

        let weights = search_support::load_weights(self.graph, &all_results);
        let merged = search_support::deduplicate_and_rank(all_results, top_k, &weights);
        let pointers = search_support::results_to_pointers(&merged, mode);
        let response = PointerResponse::build(pointers, 0);
        search_support::insert_into_cache(&self.search_cache, cache_key, response.clone());
        Ok(response)
    }

    pub fn fetch(&self, node_id: &str) -> Result<Option<FetchResponse>> {
        let Some(node) = self.graph.get_node(node_id)? else {
            return Ok(None);
        };

        let content = search_support::read_node_content_cached(&self.fetch_cache, &node)?;
        let stale = search_support::detect_staleness(&node, &content);
        let token_count = estimate_tokens(&content);

        Ok(Some(FetchResponse {
            pointer_id: node.id,
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
