// tools/hermes-engine/src/memory_query.rs
//
// MCP tool implementations for semantic memory querying:
//   - hermes_query_memory: embed a query, search Qdrant `semantic_memory`, return top-N chunks
//   - hermes_get_core_facts: read memory/CORE_FACTS.md and return its contents
//
// Qdrant URL is read from QDRANT_URL env var (default: http://localhost:6333).

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::Path;

use crate::embedding::EmbeddingGenerator;

const DEFAULT_QDRANT_URL: &str = "http://localhost:6333";
const COLLECTION_NAME: &str = "semantic_memory";
pub const DEFAULT_QUERY_LIMIT: usize = 5;

// ---------------------------------------------------------------------------
// Internal Qdrant response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct SearchResponse {
    result: Vec<SearchHit>,
}

#[derive(Debug, Deserialize)]
struct SearchHit {
    score: f32,
    payload: Option<HitPayload>,
}

#[derive(Debug, Deserialize)]
struct HitPayload {
    file_path: String,
    section_heading: String,
    chunk_text: String,
}

// ---------------------------------------------------------------------------
// Public tool functions
// ---------------------------------------------------------------------------

/// Embeds `query` and searches the Qdrant `semantic_memory` collection, returning
/// the top-`limit` matching chunks as a JSON object with a `chunks` array.
///
/// Each chunk entry has: `file_path`, `section_heading`, `text`, `score`.
///
/// Returns an error if Qdrant is unreachable or the collection does not exist.
pub fn tool_query_memory(query: &str, limit: usize) -> Result<String> {
    let qdrant_url = std::env::var("QDRANT_URL")
        .unwrap_or_else(|_| DEFAULT_QDRANT_URL.to_string());
    let qdrant_url = qdrant_url.trim_end_matches('/').to_string();

    // Run async embedding + Qdrant query inside a dedicated single-threaded runtime.
    // This avoids panicking if called from a sync context outside an existing tokio runtime.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("Failed to build tokio runtime for hermes_query_memory")?;

    rt.block_on(async move {
        // Step 1 — generate embedding (falls back to deterministic SHA256 if no API key set)
        let generator = EmbeddingGenerator::new()?;
        let embedding = generator
            .generate_embedding(query)
            .await
            .context("Failed to generate embedding for query")?;

        // Step 2 — search Qdrant
        let client = reqwest::Client::new();
        let search_url = format!(
            "{}/collections/{}/points/search",
            qdrant_url, COLLECTION_NAME
        );
        let body = json!({
            "vector": embedding,
            "limit": limit,
            "with_payload": true
        });

        let resp = client
            .post(&search_url)
            .json(&body)
            .send()
            .await
            .context("Failed to reach Qdrant")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Qdrant search failed (status={status}): {text}");
        }

        let search_resp: SearchResponse = resp
            .json()
            .await
            .context("Failed to parse Qdrant search response")?;

        let chunks: Vec<Value> = search_resp
            .result
            .into_iter()
            .map(|hit| {
                let p = hit.payload.unwrap_or(HitPayload {
                    file_path: "unknown".into(),
                    section_heading: "unknown".into(),
                    chunk_text: "".into(),
                });
                json!({
                    "file_path": p.file_path,
                    "section_heading": p.section_heading,
                    "text": p.chunk_text,
                    "score": hit.score
                })
            })
            .collect();

        Ok(serde_json::to_string_pretty(&json!({
            "query": query,
            "chunks": chunks,
            "limit": limit
        }))?)
    })
}

/// Variant of `tool_query_memory` that accepts a database connection for read isolation.
/// (The connection is not used but accepted for symmetry with other tools).
pub fn tool_query_memory_with_conn(_conn: &rusqlite::Connection, query: &str, limit: usize) -> Result<String> {
    tool_query_memory(query, limit)
}

/// Reads memory/CORE_FACTS.md from the project root and returns its contents.
/// Core facts are foundational context about the project's purpose and architecture.
pub fn tool_get_core_facts(project_root: &Path) -> Result<String> {
    let facts_path = project_root.join("memory").join("CORE_FACTS.md");
    
    if !facts_path.exists() {
        return Ok(json!({
            "status": "not_found",
            "message": "memory/CORE_FACTS.md does not exist in this project."
        }).to_string());
    }

    let content = std::fs::read_to_string(&facts_path)
        .with_context(|| format!("Failed to read {}", facts_path.display()))?;

    Ok(json!({
        "status": "ok",
        "path": "memory/CORE_FACTS.md",
        "content": content
    }).to_string())
}

/// Variant of `tool_get_core_facts` that accepts a database connection for read isolation.
pub fn tool_get_core_facts_with_conn(_conn: &rusqlite::Connection, project_root: &Path) -> Result<String> {
    tool_get_core_facts(project_root)
}
