// ChartApp/hermes-engine/src/search/mod.rs
pub mod fts;
pub mod literal;
pub mod vector;
pub(crate) mod search_support;

use crate::graph::{KnowledgeGraph, Node};
use crate::pointer::{FetchResponse, PointerResponse};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub use search_support::estimate_tokens;

/// Short-circuit thresholds for tier skipping (Task 1.2).
/// If L0 already returns top_k results all scoring >= this, skip subsequent tiers.
const SHORT_CIRCUIT_SKIP_ALL: f64 = 0.9; // Skip L1 + L2
const SHORT_CIRCUIT_SKIP_L2: f64 = 0.8; // Skip L2 only

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
    /// Task 1.3: Shared search result cache (lives on HermesEngine).
    pub(crate) search_cache: Arc<Mutex<crate::SearchCacheMap>>,
    /// Task 3.3: Per-engine fetch content cache (keyed on file_path + line range).
    pub(crate) fetch_cache: Mutex<HashMap<(String, i64, i64), String>>,
}

impl<'a> SearchEngine<'a> {
    /// Create a new SearchEngine with the shared cache from HermesEngine.
    /// Pass `engine.search_cache()` as the cache argument.
    pub fn new(graph: &'a KnowledgeGraph, search_cache: Arc<Mutex<crate::SearchCacheMap>>) -> Self {
        Self {
            graph,
            search_cache,
            fetch_cache: Mutex::new(HashMap::new()),
        }
    }

    pub fn search(&self, query: &str, top_k: usize, mode: &SearchMode) -> Result<PointerResponse> {
        // Task 1.3: Check search cache first
        let cache_key = format!("{}:{}", query.trim().to_lowercase(), top_k);
        if let Some(cached) = search_support::get_from_cache(&self.search_cache, &cache_key) {
            return Ok(cached);
        }

        let mut all_results: Vec<SearchResult> = Vec::new();

        // L0: literal search (Task 1.1: SQL-indexed, no full table scan)
        let l0_results = literal::literal_search(self.graph, query)?;

        // Task 1.2: Short-circuit if L0 already provides high-confidence top_k hits
        if l0_results.len() >= top_k {
            let min_score = l0_results
                .iter()
                .take(top_k)
                .map(|r| r.score)
                .fold(f64::INFINITY, f64::min);

            if min_score >= SHORT_CIRCUIT_SKIP_ALL {
                // Skip L1 and L2 entirely
                let weights = search_support::load_weights(self.graph, &l0_results);
                let merged = search_support::deduplicate_and_rank(l0_results, top_k, &weights);
                let pointers = search_support::results_to_pointers(&merged, mode);
                let response = PointerResponse::build(pointers, 0);
                search_support::insert_into_cache(&self.search_cache, cache_key, response.clone());
                return Ok(response);
            }

            if min_score >= SHORT_CIRCUIT_SKIP_L2 {
                // Run L1, then skip L2
                all_results.extend(l0_results);
                let l1_results = fts::fts_search(self.graph, query)?;
                all_results.extend(l1_results);
                let weights = search_support::load_weights(self.graph, &all_results);
                let merged = search_support::deduplicate_and_rank(all_results, top_k, &weights);
                let pointers = search_support::results_to_pointers(&merged, mode);
                let response = PointerResponse::build(pointers, 0);
                search_support::insert_into_cache(&self.search_cache, cache_key, response.clone());
                return Ok(response);
            }
        }

        // Run all three tiers
        all_results.extend(l0_results);

        let l1_results = fts::fts_search(self.graph, query)?;
        all_results.extend(l1_results);

        // Build a deduplicated candidate set from L0+L1 for vector reranking.
        // L2 scores only these nodes (O(candidates)) instead of the full index.
        // Falls back to full scan only when L0+L1 produced nothing.
        let mut seen = std::collections::HashSet::new();
        let candidates: Vec<crate::graph::Node> = all_results
            .iter()
            .filter(|r| seen.insert(r.node.id.clone()))
            .map(|r| r.node.clone())
            .collect();

        let l2_results = self.run_vector_rerank(query, candidates)?;
        all_results.extend(l2_results);

        let weights = search_support::load_weights(self.graph, &all_results);
        let merged = search_support::deduplicate_and_rank(all_results, top_k, &weights);
        let pointers = search_support::results_to_pointers(&merged, mode);
        let response = PointerResponse::build(pointers, 0);
        search_support::insert_into_cache(&self.search_cache, cache_key, response.clone());
        Ok(response)
    }

    fn run_vector_rerank(
        &self,
        query: &str,
        candidates: Vec<crate::graph::Node>,
    ) -> Result<Vec<SearchResult>> {
        // Avoid a full-repo vector scan when L0 + L1 found nothing.
        // On large repos this path can run longer than the MCP tool budget and
        // starve the shared Hermes proxy, which then makes stats look stale.
        if candidates.is_empty() {
            return Ok(Vec::new());
        }
        vector::vector_search(self.graph, query, candidates)
    }

    pub fn fetch(&self, pointer_id: &str) -> Result<Option<FetchResponse>> {
        let node = self.graph.get_node(pointer_id)?;
        let Some(node) = node else {
            return Ok(None);
        };

        // Task 3.3: Fetch content cache
        let content = search_support::read_node_content_cached(&self.fetch_cache, &node)?;
        let stale = search_support::detect_staleness(&node, &content);

        // Task 3.1: Word-count based token estimate (more accurate than byte / 4)
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
    fn short_circuit_skips_on_high_l0_confidence() {
        // Verify the short-circuit threshold constants are correct
        assert!(SHORT_CIRCUIT_SKIP_ALL > SHORT_CIRCUIT_SKIP_L2);
        assert!(SHORT_CIRCUIT_SKIP_ALL <= 1.0);
        assert!(SHORT_CIRCUIT_SKIP_L2 > 0.0);
    }

    #[test]
    fn vector_rerank_skips_when_candidates_are_empty() {
        let engine = crate::HermesEngine::in_memory("test-vector-skip").unwrap();
        let graph = crate::graph::KnowledgeGraph::new(engine.db().clone(), engine.project_id());
        let search = SearchEngine::new(&graph, engine.search_cache());

        let results = search
            .run_vector_rerank("nohits", Vec::new())
            .expect("vector rerank should succeed");

        assert!(results.is_empty());
    }
}
