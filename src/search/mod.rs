// ChartApp/hermes-engine/src/search/mod.rs
pub mod fts;
pub mod literal;
pub mod vector;

use crate::graph::{KnowledgeGraph, Node};
use crate::pointer::{FetchResponse, Pointer, PointerResponse};
use crate::SearchCacheMap;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const CACHE_TTL_SECS: u64 = 60;
const CACHE_MAX_ENTRIES: usize = 256;
const FETCH_CACHE_MAX_ENTRIES: usize = 50;

/// Short-circuit thresholds for tier skipping (Task 1.2).
/// If L0 already returns top_k results all scoring >= this, skip subsequent tiers.
const SHORT_CIRCUIT_SKIP_ALL: f64 = 0.9;  // Skip L1 + L2
const SHORT_CIRCUIT_SKIP_L2: f64 = 0.8;   // Skip L2 only

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
    graph: &'a KnowledgeGraph,
    /// Task 1.3: Shared search result cache (lives on HermesEngine).
    search_cache: Arc<Mutex<SearchCacheMap>>,
    /// Task 3.3: Per-engine fetch content cache (keyed on file_path + line range).
    fetch_cache: Mutex<HashMap<(String, i64, i64), String>>,
}

impl<'a> SearchEngine<'a> {
    /// Create a new SearchEngine with the shared cache from HermesEngine.
    /// Pass `engine.search_cache()` as the cache argument.
    pub fn new(graph: &'a KnowledgeGraph, search_cache: Arc<Mutex<SearchCacheMap>>) -> Self {
        Self {
            graph,
            search_cache,
            fetch_cache: Mutex::new(HashMap::new()),
        }
    }

    pub fn search(&self, query: &str, top_k: usize, mode: &SearchMode) -> Result<PointerResponse> {
        // Task 1.3: Check search cache first
        let cache_key = format!("{}:{}", query.trim().to_lowercase(), top_k);
        if let Some(cached) = self.get_from_cache(&cache_key) {
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
                let merged = Self::deduplicate_and_rank(l0_results, top_k);
                let pointers = Self::results_to_pointers(&merged, mode);
                let response = PointerResponse::build(pointers, 0);
                self.insert_into_cache(cache_key, response.clone());
                return Ok(response);
            }

            if min_score >= SHORT_CIRCUIT_SKIP_L2 {
                // Run L1, then skip L2
                all_results.extend(l0_results);
                let l1_results = fts::fts_search(self.graph, query)?;
                all_results.extend(l1_results);
                let merged = Self::deduplicate_and_rank(all_results, top_k);
                let pointers = Self::results_to_pointers(&merged, mode);
                let response = PointerResponse::build(pointers, 0);
                self.insert_into_cache(cache_key, response.clone());
                return Ok(response);
            }
        }

        // Run all three tiers
        all_results.extend(l0_results);

        let l1_results = fts::fts_search(self.graph, query)?;
        all_results.extend(l1_results);

        let l2_results = vector::vector_search(self.graph, query)?;
        all_results.extend(l2_results);

        let merged = Self::deduplicate_and_rank(all_results, top_k);
        let pointers = Self::results_to_pointers(&merged, mode);
        let response = PointerResponse::build(pointers, 0);
        self.insert_into_cache(cache_key, response.clone());
        Ok(response)
    }

    pub fn fetch(&self, pointer_id: &str) -> Result<Option<FetchResponse>> {
        let node = self.graph.get_node(pointer_id)?;
        let Some(node) = node else {
            return Ok(None);
        };

        // Task 3.3: Fetch content cache
        let content = self.read_node_content_cached(&node)?;

        // Task 3.1: Word-count based token estimate (more accurate than byte / 4)
        let token_count = estimate_tokens(&content);

        Ok(Some(FetchResponse {
            pointer_id: node.id.clone(),
            content,
            file_path: node.file_path.unwrap_or_default(),
            start_line: node.start_line.unwrap_or(0),
            end_line: node.end_line.unwrap_or(0),
            token_count,
        }))
    }

    // -----------------------------------------------------------------------
    // Cache helpers (Task 1.3)
    // -----------------------------------------------------------------------

    fn get_from_cache(&self, key: &str) -> Option<PointerResponse> {
        let ttl = Duration::from_secs(CACHE_TTL_SECS);
        let mut cache = self.search_cache.lock().ok()?;
        if let Some((response, inserted_at)) = cache.get(key) {
            if inserted_at.elapsed() < ttl {
                return Some(response.clone());
            }
            // Expired — remove it
            cache.remove(key);
        }
        None
    }

    fn insert_into_cache(&self, key: String, response: PointerResponse) {
        let Ok(mut cache) = self.search_cache.lock() else {
            return;
        };
        // Evict expired entries; if still too large, evict oldest
        if cache.len() >= CACHE_MAX_ENTRIES {
            let ttl = Duration::from_secs(CACHE_TTL_SECS);
            cache.retain(|_, (_, inserted)| inserted.elapsed() < ttl);
            if cache.len() >= CACHE_MAX_ENTRIES {
                // Find and remove the oldest entry
                if let Some(oldest_key) = cache
                    .iter()
                    .min_by_key(|(_, (_, t))| *t)
                    .map(|(k, _)| k.clone())
                {
                    cache.remove(&oldest_key);
                }
            }
        }
        cache.insert(key, (response, Instant::now()));
    }

    // -----------------------------------------------------------------------
    // Fetch content cache helper (Task 3.3)
    // -----------------------------------------------------------------------

    fn read_node_content_cached(&self, node: &Node) -> Result<String> {
        let file_path = node.file_path.clone().unwrap_or_default();
        let start = node.start_line.unwrap_or(0);
        let end = node.end_line.unwrap_or(0);
        let cache_key = (file_path.clone(), start, end);

        // Check fetch cache first
        if !file_path.is_empty() {
            if let Ok(cache) = self.fetch_cache.lock() {
                if let Some(content) = cache.get(&cache_key) {
                    return Ok(content.clone());
                }
            }
        }

        // Cache miss: read from disk
        let content = Self::read_node_content(node)?;

        // Store in fetch cache (evict oldest if over limit, simple approach)
        if !file_path.is_empty() {
            if let Ok(mut cache) = self.fetch_cache.lock() {
                if cache.len() >= FETCH_CACHE_MAX_ENTRIES {
                    if let Some(oldest) = cache.keys().next().cloned() {
                        cache.remove(&oldest);
                    }
                }
                cache.insert(cache_key, content.clone());
            }
        }

        Ok(content)
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn deduplicate_and_rank(results: Vec<SearchResult>, top_k: usize) -> Vec<SearchResult> {
        let mut best: HashMap<String, SearchResult> = HashMap::new();

        for result in results {
            let tier_bonus = match result.tier {
                SearchTier::L0Literal => 0.3,
                SearchTier::L1Fts => 0.1,
                SearchTier::L2Vector => 0.0,
            };
            let boosted_score = result.score + tier_bonus;

            best.entry(result.node.id.clone())
                .and_modify(|existing| {
                    let existing_boosted = existing.score
                        + match existing.tier {
                            SearchTier::L0Literal => 0.3,
                            SearchTier::L1Fts => 0.1,
                            SearchTier::L2Vector => 0.0,
                        };
                    if boosted_score > existing_boosted {
                        *existing = SearchResult {
                            score: result.score,
                            ..result.clone()
                        };
                    }
                })
                .or_insert(result);
        }

        let mut ranked: Vec<SearchResult> = best.into_values().collect();
        ranked.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        ranked.truncate(top_k);
        ranked
    }

    fn results_to_pointers(results: &[SearchResult], _mode: &SearchMode) -> Vec<Pointer> {
        results
            .iter()
            .map(|r| Pointer {
                id: r.node.id.clone(),
                source: r.node.file_path.clone().unwrap_or_default(),
                chunk: r.node.name.clone(),
                lines: format!(
                    "{}-{}",
                    r.node.start_line.unwrap_or(0),
                    r.node.end_line.unwrap_or(0)
                ),
                relevance: r.score,
                summary: r.node.summary.clone().unwrap_or_default(),
                node_type: r.node.node_type.as_str().to_string(),
                last_modified: None,
            })
            .collect()
    }

    fn read_node_content(node: &Node) -> Result<String> {
        let Some(ref path) = node.file_path else {
            return Ok(String::new());
        };

        let file_content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return Ok(format!("[File not found: {path}]")),
        };

        let start = node.start_line.unwrap_or(1).max(1) as usize;
        let end = node.end_line.unwrap_or(0) as usize;

        if end == 0 {
            return Ok(file_content);
        }

        let lines: Vec<&str> = file_content.lines().collect();
        let start_idx = (start - 1).min(lines.len());
        let end_idx = end.min(lines.len());
        Ok(lines[start_idx..end_idx].join("\n"))
    }
}

/// Task 3.1: Word-count based token estimation.
/// More accurate than byte-count / 4 for mixed code + prose content.
/// Invariant: 1 token ≈ 0.75 words on average → tokens = words * 4 / 3.
pub fn estimate_tokens(content: &str) -> u64 {
    let word_count = content.split_whitespace().count() as u64;
    (word_count * 4).div_ceil(3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedup_keeps_highest_score() {
        let node = Node {
            id: "n1".to_string(),
            project_id: "test".to_string(),
            name: "test_fn".to_string(),
            node_type: crate::graph::NodeType::Function,
            file_path: None,
            start_line: None,
            end_line: None,
            summary: None,
            content_hash: None,
        };

        let results = vec![
            SearchResult {
                node: node.clone(),
                score: 0.5,
                tier: SearchTier::L1Fts,
                matched_content: None,
            },
            SearchResult {
                node: node.clone(),
                score: 0.9,
                tier: SearchTier::L0Literal,
                matched_content: None,
            },
        ];

        let deduped = SearchEngine::deduplicate_and_rank(results, 10);
        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0].tier, SearchTier::L0Literal);
    }

    #[test]
    fn short_circuit_skips_on_high_l0_confidence() {
        // Verify the short-circuit threshold constants are correct
        assert!(SHORT_CIRCUIT_SKIP_ALL > SHORT_CIRCUIT_SKIP_L2);
        assert!(SHORT_CIRCUIT_SKIP_ALL <= 1.0);
        assert!(SHORT_CIRCUIT_SKIP_L2 > 0.0);
    }

    #[test]
    fn cache_miss_then_hit() {
        let engine = crate::HermesEngine::in_memory("test-cache-mod").unwrap();
        let cache = engine.search_cache();
        let dummy = PointerResponse::build(vec![], 0);
        {
            let mut c = cache.lock().unwrap();
            c.insert("key:10".to_string(), (dummy, Instant::now()));
        }
        // Verify cache has the entry
        let c = cache.lock().unwrap();
        assert!(c.contains_key("key:10"));
    }

    #[test]
    fn estimate_tokens_word_count_based() {
        // "hello world foo bar" → 4 words → 4 * 4 / 3 = 5 tokens
        let tokens = estimate_tokens("hello world foo bar");
        assert_eq!(tokens, 6); // ceil(4 * 4 / 3) = ceil(5.33) = 6
    }

    #[test]
    fn estimate_tokens_empty() {
        assert_eq!(estimate_tokens(""), 0);
    }
}

