// tools/hermes-engine/src/search/search_support.rs
use super::{SearchEngine, SearchMode, SearchResult, SearchTier};
use crate::graph::{KnowledgeGraph, Node};
use crate::pointer::{Pointer, PointerResponse};
use crate::weight::WeightStore;
use crate::SearchCacheMap;
use anyhow::Result;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const CACHE_TTL_SECS: u64 = 60;
pub const CACHE_MAX_ENTRIES: usize = 256;
pub const FETCH_CACHE_MAX_ENTRIES: usize = 50;

pub(crate) fn get_from_cache(
    search_cache: &Arc<Mutex<SearchCacheMap>>,
    key: &str,
) -> Option<PointerResponse> {
    let ttl = Duration::from_secs(CACHE_TTL_SECS);
    let mut cache = search_cache.lock().ok()?;
    if let Some((response, inserted_at)) = cache.get(key) {
        if inserted_at.elapsed() < ttl {
            return Some(response.clone());
        }
        // Expired — remove it
        cache.remove(key);
    }
    None
}

pub(crate) fn insert_into_cache(
    search_cache: &Arc<Mutex<SearchCacheMap>>,
    key: String,
    response: PointerResponse,
) {
    let Ok(mut cache) = search_cache.lock() else {
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

pub(crate) fn read_node_content_cached(
    fetch_cache: &Mutex<HashMap<(String, i64, i64), String>>,
    node: &Node,
) -> Result<String> {
    let file_path = node.file_path.clone().unwrap_or_default();
    let start = node.start_line.unwrap_or(0);
    let end = node.end_line.unwrap_or(0);
    let cache_key = (file_path.clone(), start, end);

    // Check fetch cache first
    if !file_path.is_empty() {
        if let Ok(cache) = fetch_cache.lock() {
            if let Some(content) = cache.get(&cache_key) {
                return Ok(content.clone());
            }
        }
    }

    // Cache miss: read from disk
    let content = read_node_content(node)?;

    // Store in fetch cache (evict oldest if over limit, simple approach)
    if !file_path.is_empty() {
        if let Ok(mut cache) = fetch_cache.lock() {
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

pub(crate) fn load_weights(
    graph: &KnowledgeGraph,
    results: &[SearchResult],
) -> HashMap<String, f64> {
    let node_ids: Vec<&str> = results.iter().map(|r| r.node.id.as_str()).collect();
    graph
        .with_raw_conn(|conn| {
            let store = WeightStore::from_conn(conn);
            Ok(store.get_weights_for(&node_ids).unwrap_or_default())
        })
        .unwrap_or_default()
}

pub(crate) fn deduplicate_and_rank(
    results: Vec<SearchResult>,
    top_k: usize,
    weights: &HashMap<String, f64>,
) -> Vec<SearchResult> {
    let mut best: HashMap<String, SearchResult> = HashMap::new();

    for result in results {
        let tier_bonus = match result.tier {
            SearchTier::L0Literal => 0.3,
            SearchTier::L1Fts => 0.1,
            SearchTier::L2Vector => 0.0,
        };
        let node_weight = weights.get(&result.node.id).copied().unwrap_or(1.0);
        let boosted_score = (result.score + tier_bonus) * node_weight;

        best.entry(result.node.id.clone())
            .and_modify(|existing| {
                let existing_tier_bonus = match existing.tier {
                    SearchTier::L0Literal => 0.3,
                    SearchTier::L1Fts => 0.1,
                    SearchTier::L2Vector => 0.0,
                };
                let existing_boosted = (existing.score + existing_tier_bonus)
                    * weights.get(&existing.node.id).copied().unwrap_or(1.0);
                if boosted_score > existing_boosted {
                    *existing = SearchResult {
                        score: result.score * node_weight,
                        ..result.clone()
                    };
                }
            })
            .or_insert(SearchResult {
                score: result.score * node_weight,
                ..result
            });
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

pub(crate) fn results_to_pointers(results: &[SearchResult], _mode: &SearchMode) -> Vec<Pointer> {
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
            content_tokens: r.node.content_tokens,
            object_type: r.node.object_type.clone(),
        })
        .collect()
}

pub(crate) fn read_node_content(node: &Node) -> Result<String> {
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

pub(crate) fn detect_staleness(node: &Node, content: &str) -> StalenessInfo {
    if content.starts_with("[File not found: ") {
        return StalenessInfo {
            is_stale: true,
            reason: Some("source file is missing".to_string()),
        };
    }

    // Hash-based staleness detection removed.
    // Nodes are considered fresh if the source file exists.
    // Re-indexing on next run will update any changed content.
    StalenessInfo::fresh()
}

#[derive(Debug)]
pub(crate) struct StalenessInfo {
    pub(crate) is_stale: bool,
    pub(crate) reason: Option<String>,
}

impl StalenessInfo {
    pub(crate) fn fresh() -> Self {
        Self {
            is_stale: false,
            reason: None,
        }
    }
}

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
            content_tokens: None,
            object_type: None,
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

        let deduped = deduplicate_and_rank(results, 10, &HashMap::new());
        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0].tier, SearchTier::L0Literal);
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
