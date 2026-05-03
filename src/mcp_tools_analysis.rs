// tools/hermes-engine/src/mcp_tools_analysis.rs
//
// Duplicate-detection and search-miss analysis tools.
// Extracted from mcp_tools.rs for size compliance.

use anyhow::Result;
use serde_json::json;

use crate::{accounting::Accountant, graph::KnowledgeGraph, HermesEngine};

/// Scan the stored symbol embeddings for duplicates of the given signature.
/// Returns a JSON object matching the spec in docs/EMBEDDING_SCANNER_SPEC.md.
pub fn tool_scan_duplicates(engine: &HermesEngine, signature: &str) -> Result<String> {
    let graph = KnowledgeGraph::new(engine.read_db().clone(), engine.project_id());

    // compute embedding for query (may be deterministic fallback)
    let query_emb = crate::embedding::embed_text(signature);
    let mut matches = Vec::new();
    let mut score_max = 0.0f32;

    if !query_emb.is_empty() {
        for (sym, file, _sig, snippet, emb) in graph.get_all_symbol_embeddings()? {
            let score = cosine_similarity(&query_emb, &emb);
            if score > score_max {
                score_max = score;
            }
            if score >= 0.80 {
                matches.push(json!({
                    "symbol_name": sym,
                    "file_path": file,
                    "similarity_score": score,
                    "snippet": snippet
                }));
            }
        }
    }

    let result = json!({
        "has_duplicates": !matches.is_empty(),
        "matches": matches,
        "score_max": score_max
    });
    let acct = Accountant::new(engine.db().clone(), engine.project_id(), engine.session_id());
    let ptr_tokens = (matches.len() as u64).saturating_mul(80) + 30;
    let _ = acct.record_query(
        &format!("scan_duplicates:{}", &signature[..signature.len().min(40)]),
        ptr_tokens,
        0,
        ptr_tokens.saturating_mul(15),
    );
    Ok(serde_json::to_string_pretty(&result)?)
}

/// Compute cosine similarity between two vectors. If either vector has zero
/// magnitude the similarity is defined to be 0.0 (avoids NaNs).
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let len = std::cmp::min(a.len(), b.len());
    let mut dot = 0.0;
    let mut na = 0.0;
    let mut nb = 0.0;
    for i in 0..len {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    let na = na.sqrt();
    let nb = nb.sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

/// Returns a post-mortem report of all searches that returned zero results.
///
/// `since_days` — optional window in days (None = all time).
/// `top_k` — how many top repeated queries to surface in the aggregation.
pub fn tool_search_misses(
    engine: &HermesEngine,
    since_days: Option<u64>,
    top_k: usize,
) -> Result<String> {
    let since = since_days.map(|d| std::time::Duration::from_secs(d * 86_400));
    let acct = Accountant::new(
        engine.read_db().clone(),
        engine.project_id(),
        engine.session_id(),
    );
    let misses = acct.query_search_misses(100, since)?;
    let top = acct.top_missed_queries(top_k, since)?;

    let since_label = since_days
        .map(|d| format!("last {d}d"))
        .unwrap_or_else(|| "all time".to_string());

    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "since": since_label,
        "total_misses": misses.len(),
        "top_missed_queries": top.iter().map(|(q, c)| serde_json::json!({"query": q, "count": c})).collect::<Vec<_>>(),
        "recent_misses": misses.iter().map(|m| serde_json::json!({
            "id": m.id,
            "query": m.query,
            "effective_query": m.effective_query,
            "goal": m.goal,
            "source": m.source,
            "created_at": m.created_at,
        })).collect::<Vec<_>>(),
    }))?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedding::EmbeddingGenerator;
    use crate::{graph::KnowledgeGraph, HermesEngine};
    use sha2::{Digest, Sha256};
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    struct EnvVarGuard {
        key: &'static str,
        old: Option<String>,
    }

    impl EnvVarGuard {
        fn clear(key: &'static str) -> Self {
            let old = std::env::var(key).ok();
            std::env::remove_var(key);
            Self { key, old }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(old) = &self.old {
                std::env::set_var(self.key, old);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    #[test]
    fn test_scan_duplicates_no_embeddings() {
        let _guard = env_lock().lock().expect("env lock");
        let engine = HermesEngine::in_memory("test-scan-none").unwrap();
        let result: serde_json::Value =
            serde_json::from_str(&tool_scan_duplicates(&engine, "foo").unwrap()).unwrap();
        assert_eq!(result["has_duplicates"], false);
        assert!(result["matches"].as_array().unwrap().is_empty());
        assert_eq!(result["score_max"], 0.0);
    }

    #[test]
    fn test_scan_duplicates_with_match() {
        let _guard = env_lock().lock().expect("env lock");
        let _k1 = EnvVarGuard::clear("OPENAI_API_KEY");
        let _k2 = EnvVarGuard::clear("GEMINI_API_KEY");
        let _k3 = EnvVarGuard::clear("HERMES_LLM_GATEWAY_URL");
        let _k4 = EnvVarGuard::clear("LMSTUDIO_URL");
        let _k5 = EnvVarGuard::clear("LMSTUDIO_EMBED_URL");
        let engine = HermesEngine::in_memory("test-scan-match").unwrap();
        let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());

        let sig = "fn example()";
        let emb = crate::embedding::deterministic_embedding(sig, EmbeddingGenerator::dimension());
        let id = {
            let mut hasher = Sha256::new();
            hasher.update("/path/file.rs".as_bytes());
            hasher.update(b"::");
            hasher.update(sig.as_bytes());
            hex::encode(hasher.finalize())
        };
        // insert directly using graph helper
        graph
            .upsert_symbol_embedding(
                &id,
                "example",
                "/path/file.rs",
                "rust",
                sig,
                "example snippet",
                &emb,
            )
            .unwrap();

        let result: serde_json::Value =
            serde_json::from_str(&tool_scan_duplicates(&engine, sig).unwrap()).unwrap();
        assert_eq!(result["has_duplicates"], true);
        let arr = result["matches"].as_array().unwrap();
        assert!(!arr.is_empty());
        assert!(result["score_max"].as_f64().unwrap() > 0.0);
    }
}
