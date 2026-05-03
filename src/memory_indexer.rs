// tools/hermes-engine/src/memory_indexer.rs
//
// Indexes all .md files under memory/ by H2 section into a Qdrant
// `semantic_memory` collection. Uses the llm-gateway embedding endpoint
// (HERMES_LLM_GATEWAY_URL env var) and the Qdrant REST API
// (QDRANT_URL env var, defaults to http://localhost:6333).
//
// The indexer is incremental: each chunk's stable ID is derived from its
// file_path + section_heading; upserts are skipped when the stored
// `last_modified` Unix timestamp already matches the file's mtime.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

use crate::embedding::EmbeddingGenerator;

/// Default Qdrant REST API port.
const DEFAULT_QDRANT_URL: &str = "http://localhost:6333";
const COLLECTION_NAME: &str = "semantic_memory";
/// Vector dimension produced by EmbeddingGenerator.
const VECTOR_DIM: u64 = 1024;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A single chunk extracted from a memory markdown file.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryChunk {
    pub file_path: String,
    pub section_heading: String,
    pub chunk_text: String,
    /// Unix timestamp (seconds) of the source file's mtime.
    pub last_modified: i64,
}

/// Summary statistics returned by `MemoryIndexer::run`.
#[derive(Debug, Default)]
pub struct IndexStats {
    pub total_chunks: usize,
    pub upserted: usize,
    pub skipped: usize,
}

/// Payload stored alongside each Qdrant point.
#[derive(Debug, Serialize, Deserialize)]
pub struct ChunkPayload {
    pub file_path: String,
    pub section_heading: String,
    pub chunk_text: String,
    pub last_modified: i64,
}

// ---------------------------------------------------------------------------
// Pure helper functions (testable without network)
// ---------------------------------------------------------------------------

/// Splits a markdown document into chunks by H2 (`## `) headings.
///
/// Content appearing before the first `## ` heading is collected under an
/// empty `section_heading`.  Each heading line is included at the start of
/// its chunk so that the heading text is embedded as part of the chunk.
pub fn chunk_by_h2(content: &str, file_path: &str, last_modified: i64) -> Vec<MemoryChunk> {
    let mut chunks: Vec<MemoryChunk> = Vec::new();
    let mut current_heading = String::new();
    let mut current_lines: Vec<&str> = Vec::new();

    for line in content.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            // Flush the accumulator for the previous section.
            let text = current_lines.join("\n").trim().to_string();
            if !text.is_empty() {
                chunks.push(MemoryChunk {
                    file_path: file_path.to_string(),
                    section_heading: current_heading.clone(),
                    chunk_text: text,
                    last_modified,
                });
            }
            current_heading = heading.trim().to_string();
            current_lines = vec![line]; // include the heading line in the chunk
        } else {
            current_lines.push(line);
        }
    }
    // Flush the final section.
    let text = current_lines.join("\n").trim().to_string();
    if !text.is_empty() {
        chunks.push(MemoryChunk {
            file_path: file_path.to_string(),
            section_heading: current_heading,
            chunk_text: text,
            last_modified,
        });
    }
    chunks
}

/// Deterministic u64 point ID for a chunk, based on file_path + section_heading.
///
/// Stable across runs as long as the file path and heading are unchanged.
pub fn chunk_point_id(file_path: &str, section_heading: &str) -> u64 {
    let mut h = DefaultHasher::new();
    format!("{file_path}::{section_heading}").hash(&mut h);
    h.finish()
}

/// Returns `true` when `stored_ts` matches `file_last_modified`, meaning no
/// re-embed is needed.  Extracted as a pure function for easy unit testing.
pub fn should_skip(stored_ts: Option<i64>, file_last_modified: i64) -> bool {
    stored_ts == Some(file_last_modified)
}

/// Build the JSON body for a Qdrant upsert request.
pub fn build_upsert_body(id: u64, embedding: &[f32], chunk: &MemoryChunk) -> serde_json::Value {
    serde_json::json!({
        "points": [{
            "id": id,
            "vector": embedding,
            "payload": {
                "file_path": chunk.file_path,
                "section_heading": chunk.section_heading,
                "chunk_text": chunk.chunk_text,
                "last_modified": chunk.last_modified,
            }
        }]
    })
}

// ---------------------------------------------------------------------------
// MemoryIndexer
// ---------------------------------------------------------------------------

pub struct MemoryIndexer {
    qdrant_url: String,
    generator: EmbeddingGenerator,
    client: reqwest::Client,
}

impl MemoryIndexer {
    pub fn new() -> Result<Self> {
        let qdrant_url = std::env::var("QDRANT_URL")
            .unwrap_or_else(|_| DEFAULT_QDRANT_URL.to_string());
        let generator = EmbeddingGenerator::new()?;
        Ok(Self {
            qdrant_url,
            generator,
            client: reqwest::Client::new(),
        })
    }

    /// Index all `.md` files under `memory_root` into Qdrant.
    pub async fn run(&self, memory_root: &Path) -> Result<IndexStats> {
        self.ensure_collection()
            .await
            .context("Failed to ensure Qdrant collection exists")?;

        let md_files = find_md_files(memory_root)?;
        let mut stats = IndexStats::default();

        for file_path in md_files {
            let last_modified = file_last_modified_secs(&file_path)?;
            let content = std::fs::read_to_string(&file_path)
                .with_context(|| format!("Cannot read {file_path:?}"))?;
            let display_path = file_path.to_string_lossy().to_string();
            let chunks = chunk_by_h2(&content, &display_path, last_modified);
            stats.total_chunks += chunks.len();

            for chunk in &chunks {
                let id = chunk_point_id(&chunk.file_path, &chunk.section_heading);

                let stored_ts = self.get_stored_last_modified(id).await;
                if should_skip(stored_ts, chunk.last_modified) {
                    stats.skipped += 1;
                    continue;
                }

                match self.generator.generate_embedding(&chunk.chunk_text).await {
                    Ok(embedding) => {
                        if let Err(e) = self.upsert_point(id, chunk, &embedding).await {
                            warn!(
                                "Qdrant upsert failed for {}/{}: {e}",
                                chunk.file_path, chunk.section_heading
                            );
                        } else {
                            stats.upserted += 1;
                        }
                    }
                    Err(e) => {
                        warn!(
                            "Embedding failed for {}/{}: {e}",
                            chunk.file_path, chunk.section_heading
                        );
                    }
                }
            }
        }

        info!(
            "Memory index complete — {} total, {} upserted, {} skipped",
            stats.total_chunks, stats.upserted, stats.skipped
        );
        Ok(stats)
    }

    // -----------------------------------------------------------------------
    // Qdrant REST helpers
    // -----------------------------------------------------------------------

    /// Creates the `semantic_memory` collection if it does not already exist.
    async fn ensure_collection(&self) -> Result<()> {
        let url = format!("{}/collections/{COLLECTION_NAME}", self.qdrant_url);

        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .context("Qdrant GET /collections request failed")?;

        if resp.status().is_success() {
            return Ok(()); // already exists
        }

        // Create the collection with cosine distance at 768 dimensions.
        let body = serde_json::json!({
            "vectors": {
                "size": VECTOR_DIM,
                "distance": "Cosine"
            }
        });
        let put = self
            .client
            .put(&url)
            .json(&body)
            .send()
            .await
            .context("Qdrant PUT /collections request failed")?;

        if !put.status().is_success() {
            let status = put.status();
            let text = put.text().await.unwrap_or_default();
            anyhow::bail!("Qdrant collection creation failed ({status}): {text}");
        }

        info!("Created Qdrant collection '{COLLECTION_NAME}'");
        Ok(())
    }

    /// Fetches the `last_modified` payload field for a point by its ID.
    /// Returns `None` if the point does not exist or cannot be read.
    async fn get_stored_last_modified(&self, id: u64) -> Option<i64> {
        let url = format!(
            "{}/collections/{COLLECTION_NAME}/points/get",
            self.qdrant_url
        );
        let body = serde_json::json!({ "ids": [id], "with_payload": true });
        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .ok()?;

        if !resp.status().is_success() {
            return None;
        }

        let json: serde_json::Value = resp.json().await.ok()?;
        json["result"]
            .as_array()?
            .first()?
            ["payload"]["last_modified"]
            .as_i64()
    }

    /// Upserts a single point into Qdrant.
    async fn upsert_point(&self, id: u64, chunk: &MemoryChunk, embedding: &[f32]) -> Result<()> {
        let url = format!(
            "{}/collections/{COLLECTION_NAME}/points",
            self.qdrant_url
        );
        let body = build_upsert_body(id, embedding, chunk);
        let resp = self
            .client
            .put(&url)
            .json(&body)
            .send()
            .await
            .context("Qdrant PUT /points request failed")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Qdrant upsert failed ({status}): {text}");
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// File-system helpers
// ---------------------------------------------------------------------------

fn find_md_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    if !root.exists() {
        return Ok(files);
    }
    collect_md_files(root, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_md_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir)
        .with_context(|| format!("Cannot read directory {dir:?}"))?
    {
        let entry = entry.context("Directory entry error")?;
        let path = entry.path();
        if path.is_dir() {
            collect_md_files(&path, out)?;
        } else if path.extension().map(|e| e == "md").unwrap_or(false) {
            out.push(path);
        }
    }
    Ok(())
}

fn file_last_modified_secs(path: &Path) -> Result<i64> {
    let meta = std::fs::metadata(path)
        .with_context(|| format!("Cannot stat {path:?}"))?;
    Ok(meta
        .modified()
        .context("mtime not supported on this platform")?
        .duration_since(std::time::UNIX_EPOCH)
        .context("mtime is before UNIX epoch")?
        .as_secs() as i64)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- chunk splitting ---

    #[test]
    fn chunk_by_h2_splits_on_h2_headings() {
        let content = "Preamble text.\n## First\nFirst body.\n## Second\nSecond body.";
        let chunks = chunk_by_h2(content, "memory/test.md", 1000);
        assert_eq!(chunks.len(), 3, "preamble + two H2 sections");
        assert_eq!(chunks[0].section_heading, "");
        assert!(chunks[0].chunk_text.contains("Preamble"));
        assert_eq!(chunks[1].section_heading, "First");
        assert!(chunks[1].chunk_text.contains("First body"));
        assert_eq!(chunks[2].section_heading, "Second");
        assert!(chunks[2].chunk_text.contains("Second body"));
    }

    #[test]
    fn chunk_by_h2_handles_no_headings() {
        let content = "Just some plain text.\nWith two lines.";
        let chunks = chunk_by_h2(content, "memory/plain.md", 42);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].section_heading, "");
        assert!(chunks[0].chunk_text.contains("plain text"));
    }

    #[test]
    fn chunk_by_h2_empty_content_produces_no_chunks() {
        let chunks = chunk_by_h2("", "memory/empty.md", 0);
        assert!(chunks.is_empty());
    }

    #[test]
    fn chunk_by_h2_carries_last_modified() {
        let content = "## Only\nBody.";
        let chunks = chunk_by_h2(content, "memory/x.md", 9999);
        assert_eq!(chunks[0].last_modified, 9999);
    }

    #[test]
    fn chunk_by_h2_ignores_h1_and_h3_headings() {
        let content = "# H1 heading\nPreamble.\n### H3 heading\nText.";
        let chunks = chunk_by_h2(content, "memory/x.md", 1);
        // No H2 found → single chunk for all content
        assert_eq!(chunks.len(), 1);
    }

    // --- chunk_point_id ---

    #[test]
    fn chunk_point_id_is_deterministic() {
        let a = chunk_point_id("memory/foo.md", "Introduction");
        let b = chunk_point_id("memory/foo.md", "Introduction");
        assert_eq!(a, b);
    }

    #[test]
    fn chunk_point_id_differs_for_different_inputs() {
        let a = chunk_point_id("memory/foo.md", "Introduction");
        let b = chunk_point_id("memory/foo.md", "Conclusion");
        assert_ne!(a, b);
    }

    // --- skip-unchanged logic ---

    #[test]
    fn should_skip_when_timestamps_match() {
        assert!(should_skip(Some(1_700_000_000), 1_700_000_000));
    }

    #[test]
    fn should_not_skip_when_timestamps_differ() {
        assert!(!should_skip(Some(1_700_000_000), 1_700_000_001));
    }

    #[test]
    fn should_not_skip_when_no_stored_timestamp() {
        assert!(!should_skip(None, 1_700_000_000));
    }

    // --- upsert payload shape ---

    #[test]
    fn build_upsert_body_has_correct_shape() {
        let chunk = MemoryChunk {
            file_path: "memory/decisions/2026-01-01_foo.md".to_string(),
            section_heading: "Background".to_string(),
            chunk_text: "Some decision text.".to_string(),
            last_modified: 1_700_000_042,
        };
        let embedding: Vec<f32> = vec![0.1; 1024];
        let body = build_upsert_body(42, &embedding, &chunk);

        let point = &body["points"][0];
        assert_eq!(point["id"], 42u64);
        assert_eq!(point["vector"].as_array().unwrap().len(), 1024);
        let payload = &point["payload"];
        assert_eq!(payload["file_path"], "memory/decisions/2026-01-01_foo.md");
        assert_eq!(payload["section_heading"], "Background");
        assert_eq!(payload["chunk_text"], "Some decision text.");
        assert_eq!(payload["last_modified"], 1_700_000_042i64);
    }
}
