// tools/hermes-engine/src/embedding/generator.rs
use super::deterministic_embedding;
use super::provider_types::{
    EmbeddingContent, EmbeddingPart, EmbeddingRequest, EmbeddingResponse, LmStudioRequest,
    LmStudioResponse, OaRequest, OaResponse,
};
use super::routing::{resolve_lmstudio_targets, LmStudioTarget, DEFAULT_LMSTUDIO_MODEL};
use anyhow::{Context, Result};
use shared_rust::llm_gateway_client::LlmGatewayClient;
use std::env;
use std::sync::Arc;
use tokio::sync::Semaphore;

const DEFAULT_MODEL: &str = "text-embedding-004";
const DEFAULT_DIMENSION: usize = 1024;
const DEFAULT_RPM: usize = 60;
const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 10;

#[derive(Clone)]
pub struct EmbeddingGenerator {
    api_key: String,
    model: String,
    gateway_client: Option<LlmGatewayClient>,
    lmstudio_targets: Vec<LmStudioTarget>,
    client: reqwest::Client,
    rate_limiter: Arc<Semaphore>,
}

impl EmbeddingGenerator {
    pub fn new() -> Result<Self> {
        let api_key = env::var("OPENAI_API_KEY")
            .or_else(|_| env::var("GEMINI_API_KEY"))
            .unwrap_or_default();
        let model = env::var("OPENAI_EMBEDDING_MODEL")
            .or_else(|_| env::var("GEMINI_EMBEDDING_MODEL"))
            .unwrap_or_else(|_| DEFAULT_MODEL.to_string());
        let gateway_url = env::var("HERMES_LLM_GATEWAY_URL").ok();
        let gateway_key = env::var("HERMES_LLM_GATEWAY_KEY").ok();
        let lmstudio_targets = resolve_lmstudio_targets();

        let rpm: usize = env::var("EMBEDDING_RPM")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_RPM);

        let gateway_client = match (gateway_url, gateway_key) {
            (Some(url), Some(key)) => Some(LlmGatewayClient::new(
                reqwest::Client::new(),
                url,
                Some(key),
            )),
            (Some(url), None) => Some(LlmGatewayClient::new(reqwest::Client::new(), url, None)),
            (None, _) => None,
        };

        Ok(Self {
            api_key,
            model,
            gateway_client,
            lmstudio_targets,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(DEFAULT_REQUEST_TIMEOUT_SECS))
                .build()
                .context("Failed to build embedding HTTP client")?,
            rate_limiter: Arc::new(Semaphore::new(rpm)),
        })
    }

    pub fn dimension() -> usize {
        DEFAULT_DIMENSION
    }

    /// Returns the LM Studio model name resolved from env vars.
    pub fn lmstudio_model(&self) -> &str {
        self.lmstudio_targets
            .first()
            .map(|target| target.model.as_str())
            .unwrap_or(DEFAULT_LMSTUDIO_MODEL)
    }

    pub fn lmstudio_urls(&self) -> Vec<&str> {
        self.lmstudio_targets
            .iter()
            .map(|target| target.url.as_str())
            .collect()
    }

    pub fn lmstudio_targets(&self) -> Vec<(&str, &str)> {
        self.lmstudio_targets
            .iter()
            .map(|target| (target.url.as_str(), target.model.as_str()))
            .collect()
    }

    pub async fn generate_embedding(&self, text: &str) -> Result<Vec<f32>> {
        let _permit = self
            .rate_limiter
            .acquire()
            .await
            .map_err(|e| anyhow::anyhow!("Rate limiter closed: {e}"))?;

        if let Some(gateway) = &self.gateway_client {
            match gateway
                .embed(text, Some(&self.model), "hermes-engine")
                .await
            {
                Ok(result) => {
                    if let Some(first) = result.embeddings.into_iter().next() {
                        return Ok(first);
                    }
                }
                Err(e) => {
                    tracing::warn!("LLM Gateway embedding failed: {e} — falling back to LM Studio or direct provider");
                }
            }
        }

        for target in &self.lmstudio_targets {
            match self.try_lmstudio(target, text).await {
                Ok(vec) => return Ok(vec),
                Err(e) => {
                    tracing::warn!(
                        "LM Studio fallback failed for {}: {e} — trying next provider",
                        target.url
                    );
                }
            }
        }

        if self.api_key.is_empty() {
            return Ok(deterministic_embedding(
                text,
                EmbeddingGenerator::dimension(),
            ));
        }

        let url = if self.api_key.starts_with("sk-") {
            format!("https://api.openai.com/v1/embeddings")
        } else {
            format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{}:embedContent?key={}",
                self.model, self.api_key
            )
        };

        if self.api_key.starts_with("sk-") {
            let request = OaRequest {
                model: &self.model,
                input: text,
            };

            let response = self
                .client
                .post(&url)
                .bearer_auth(&self.api_key)
                .json(&request)
                .send()
                .await
                .context("Failed to call OpenAI embedding API")?;

            let status = response.status();
            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                anyhow::bail!("OpenAI embedding API returned {status}: {body}");
            }

            let parsed: OaResponse = response
                .json()
                .await
                .context("Failed to parse OpenAI embedding response")?;
            parsed
                .data
                .into_iter()
                .next()
                .map(|item| item.embedding.values)
                .ok_or_else(|| anyhow::anyhow!("OpenAI returned empty embedding data"))
        } else {
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
    }

    async fn try_lmstudio(&self, target: &LmStudioTarget, text: &str) -> Result<Vec<f32>> {
        let url = format!("{}/v1/embeddings", target.url.trim_end_matches('/'));
        let request = LmStudioRequest {
            model: &target.model,
            input: text,
        };
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            self.client.post(&url).json(&request).send(),
        )
        .await
        .map_err(|_| anyhow::anyhow!("LM Studio embedding request timed out after 10s"))?
        .context("Failed to connect to LM Studio for embedding")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("LM Studio returned {status}: {body}");
        }
        let parsed: LmStudioResponse = response
            .json()
            .await
            .context("Failed to parse LM Studio embedding response")?;
        parsed
            .data
            .into_iter()
            .next()
            .map(|e| e.embedding)
            .ok_or_else(|| anyhow::anyhow!("LM Studio returned empty embedding data"))
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
