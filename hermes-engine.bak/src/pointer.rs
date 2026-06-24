// ChartApp/hermes-engine/src/pointer.rs
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pointer {
    pub id: String,
    pub source: String,
    pub chunk: String,
    pub lines: String,
    pub relevance: f64,
    pub summary: String,
    pub node_type: String,
    pub last_modified: Option<String>,
    /// Optional stored token count for the node's content (set at ingestion).
    pub content_tokens: Option<u64>,
    /// TRACK-040 Phase 2: AST-derived object type (function, struct, enum, etc.)
    pub object_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PointerResponse {
    pub pointers: Vec<Pointer>,
    pub accounting: AccountingReport,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountingReport {
    pub pointer_tokens: u64,
    pub fetched_tokens: u64,
    pub total_tokens: u64,
    pub traditional_rag_estimate: u64,
    pub savings_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchResponse {
    pub pointer_id: String,
    pub content: String,
    pub file_path: String,
    pub start_line: i64,
    pub end_line: i64,
    pub token_count: u64,
    /// Node-level stored token count when available (ingestion-time value).
    pub content_tokens: Option<u64>,
    /// True when the current fetched content no longer matches ingestion-time hash metadata.
    #[serde(default)]
    pub is_stale: bool,
    /// Why the node was considered stale (hash mismatch, file missing, etc.).
    pub stale_reason: Option<String>,
    /// Hash captured at ingestion time for the node/chunk.
    pub stored_content_hash: Option<String>,
    /// Hash of content fetched from disk in the current request.
    pub current_content_hash: Option<String>,
}

impl Pointer {
    /// Task 3.1: Word-count based token estimation (more accurate than byte / 4).
    pub fn estimate_token_count(&self) -> u64 {
        let text = format!(
            "{} {} {} {}",
            self.source, self.chunk, self.lines, self.summary
        );
        let word_count = text.split_whitespace().count() as u64;
        // ~0.75 words per token → tokens = words * 4 / 3
        (word_count * 4).div_ceil(3) + 2 // +2 overhead for id/type fields
    }
}

impl PointerResponse {
    pub fn build(pointers: Vec<Pointer>, fetched_tokens: u64) -> Self {
        let pointer_tokens: u64 = pointers.iter().map(|p| p.estimate_token_count()).sum();

        // Feature flag: when enabled, prefer stored per-node content token counts
        // to compute the "traditional" RAG estimate. If some pointers lack the
        // stored value, fall back to per-pointer heuristic (pointer_tokens * 15
        // for the missing fraction).
        let use_real = std::env::var("HERMES_ENABLE_REAL_TRADITIONAL_ESTIMATE")
            .map(|v| v != "0" && v.to_lowercase() != "false")
            .unwrap_or(false);

        let traditional_estimate = if use_real {
            // Sum stored content_tokens where available; for pointers without
            // stored content_tokens, estimate using pointer.estimate_token_count()*15.
            let mut sum: u64 = 0;
            for p in &pointers {
                if let Some(ct) = p.content_tokens {
                    sum = sum.saturating_add(ct);
                } else {
                    sum = sum.saturating_add(p.estimate_token_count() * 15);
                }
            }
            sum
        } else {
            pointer_tokens * 15
        };

        let total = pointer_tokens + fetched_tokens;
        let savings_pct = if traditional_estimate > 0 {
            (1.0 - (total as f64 / traditional_estimate as f64)) * 100.0
        } else {
            0.0
        };

        Self {
            pointers,
            accounting: AccountingReport {
                pointer_tokens,
                fetched_tokens,
                total_tokens: total,
                traditional_rag_estimate: traditional_estimate,
                savings_pct: savings_pct.max(0.0),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        ENV_LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn pointer_token_estimation() {
        let ptr = Pointer {
            id: "abc".to_string(),
            source: "src/main.rs".to_string(),
            chunk: "fn main".to_string(),
            lines: "1-20".to_string(),
            relevance: 0.95,
            summary: "Application entry point".to_string(),
            node_type: "function".to_string(),
            last_modified: None,
            content_tokens: None,
            object_type: None,
        };
        let tokens = ptr.estimate_token_count();
        assert!(tokens > 0 && tokens < 100);
    }

    #[test]
    fn pointer_response_calculates_savings() {
        let ptrs = vec![Pointer {
            id: "1".to_string(),
            source: "src/lib.rs".to_string(),
            chunk: "struct Engine".to_string(),
            lines: "10-30".to_string(),
            relevance: 0.9,
            summary: "Main engine struct with configuration".to_string(),
            node_type: "struct".to_string(),
            last_modified: None,
            content_tokens: None,
            object_type: None,
        }];
        let resp = PointerResponse::build(ptrs, 0);
        assert!(resp.accounting.savings_pct > 0.0);
        assert!(resp.accounting.traditional_rag_estimate > resp.accounting.pointer_tokens);
    }

    fn with_env_var<T>(key: &str, val: &str, f: impl FnOnce() -> T) -> T {
        let _guard = env_lock().lock().unwrap();
        let prev = std::env::var(key).ok();
        std::env::set_var(key, val);
        let out = f();
        match prev {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
        out
    }

    #[test]
    fn pointer_response_real_traditional_estimate_uses_stored_counts() {
        with_env_var("HERMES_ENABLE_REAL_TRADITIONAL_ESTIMATE", "1", || {
            let ptrs = vec![
                Pointer {
                    id: "a".to_string(),
                    source: "src/a.rs".to_string(),
                    chunk: "a_chunk".to_string(),
                    lines: "1-10".to_string(),
                    relevance: 0.9,
                    summary: "A".to_string(),
                    node_type: "function".to_string(),
                    last_modified: None,
                    content_tokens: Some(300),
                    object_type: None,
                },
                Pointer {
                    id: "b".to_string(),
                    source: "src/b.rs".to_string(),
                    chunk: "b_chunk".to_string(),
                    lines: "1-10".to_string(),
                    relevance: 0.8,
                    summary: "B".to_string(),
                    node_type: "function".to_string(),
                    last_modified: None,
                    content_tokens: Some(200),
                    object_type: None,
                },
            ];
            let resp = PointerResponse::build(ptrs, 0);
            assert_eq!(resp.accounting.traditional_rag_estimate, 500);
        });
    }

    #[test]
    fn pointer_response_real_estimate_falls_back_for_missing() {
        with_env_var("HERMES_ENABLE_REAL_TRADITIONAL_ESTIMATE", "1", || {
            let ptr_with = Pointer {
                id: "x".to_string(),
                source: "src/x.rs".to_string(),
                chunk: "x_chunk".to_string(),
                lines: "1-2".to_string(),
                relevance: 0.9,
                summary: "X".to_string(),
                node_type: "function".to_string(),
                last_modified: None,
                content_tokens: Some(400),
                object_type: None,
            };
            let ptr_without = Pointer {
                id: "y".to_string(),
                source: "src/y.rs".to_string(),
                chunk: "y_chunk".to_string(),
                lines: "1-2".to_string(),
                relevance: 0.5,
                summary: "Y".to_string(),
                node_type: "function".to_string(),
                last_modified: None,
                content_tokens: None,
                object_type: None,
            };
            // Compute expected: stored(400) + fallback(pointer_tokens_for_y * 15)
            let fallback = ptr_without.estimate_token_count() * 15;
            let resp = PointerResponse::build(vec![ptr_with, ptr_without], 0);
            assert_eq!(resp.accounting.traditional_rag_estimate, 400 + fallback);
        });
    }
}
