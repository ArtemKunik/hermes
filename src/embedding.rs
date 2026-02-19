//! # Optional: Gemini Embedding Generator
//!
//! This module provides an `EmbeddingGenerator` that calls the Google Gemini
//! embedding API (`text-embedding-004`).  It is **not** used by the default
//! search pipeline — the built-in vector search in `search::vector` uses a
//! local token-hashing approach that requires no external service.
//!
//! You can wire this module into the ingestion or search pipeline if you want
//! higher-quality semantic embeddings.  To use it, set:
//!
//! - `GEMINI_API_KEY`           — your Google AI API key (required)
//! - `GEMINI_EMBEDDING_MODEL`   — model name (default: `text-embedding-004`)
//! - `EMBEDDING_RPM`            — rate limit in requests/min (default: 60)

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::env;
use std::sync::Arc;
use tokio::sync::Semaphore;

const DEFAULT_MODEL: &str = "text-embedding-004";
const DEFAULT_DIMENSION: usize = 768;
const DEFAULT_RPM: usize = 60;

#[derive(Clone)]
pub struct EmbeddingGenerator {
    api_key: String,
    model: String,
    client: reqwest::Client,
    rate_limiter: Arc<Semaphore>,
}

#[derive(Debug, Serialize)]
struct EmbeddingRequest {
    model: String,
    content: EmbeddingContent,
}

#[derive(Debug, Serialize)]
struct EmbeddingContent {
    parts: Vec<EmbeddingPart>,
}

#[derive(Debug, Serialize)]
struct EmbeddingPart {
    text: String,
}

#[derive(Debug, Deserialize)]
struct EmbeddingResponse {
    embedding: EmbeddingValues,
}

#[derive(Debug, Deserialize)]
struct EmbeddingValues {
    values: Vec<f32>,
}

impl EmbeddingGenerator {
    pub fn new() -> Result<Self> {
        let api_key = env::var("GEMINI_API_KEY")
            .context("GEMINI_API_KEY environment variable not set")?;
        let model = env::var("GEMINI_EMBEDDING_MODEL")
            .unwrap_or_else(|_| DEFAULT_MODEL.to_string());
        let rpm: usize = env::var("EMBEDDING_RPM")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_RPM);

        Ok(Self {
            api_key,
            model,
            client: reqwest::Client::new(),
            rate_limiter: Arc::new(Semaphore::new(rpm)),
        })
    }

    pub fn dimension() -> usize {
        DEFAULT_DIMENSION
    }

    pub async fn generate_embedding(&self, text: &str) -> Result<Vec<f32>> {
        let _permit = self.rate_limiter.acquire().await
            .map_err(|e| anyhow::anyhow!("Rate limiter closed: {e}"))?;

        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:embedContent?key={}",
            self.model, self.api_key
        );

        let request = EmbeddingRequest {
            model: format!("models/{}", self.model),
            content: EmbeddingContent {
                parts: vec![EmbeddingPart {
                    text: text.to_string(),
                }],
            },
        };

        let response = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await
            .context("Failed to call embedding API")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Embedding API returned {status}: {body}");
        }

        let parsed: EmbeddingResponse = response
            .json()
            .await
            .context("Failed to parse embedding response")?;

        Ok(parsed.embedding.values)
    }

    pub async fn generate_embeddings(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let mut results = Vec::with_capacity(texts.len());
        for text in texts {
            let embedding = self.generate_embedding(text).await?;
            results.push(embedding);
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dimension_is_768() {
        assert_eq!(EmbeddingGenerator::dimension(), 768);
    }
}
