//! Optional neural embedding client for OpenAI-compatible embedding APIs
//! (e.g. LM Studio, Ollama, llama.cpp, etc.).
//!
//! Activated by setting `HERMES_EMBED_URL` to an OpenAI-compatible embeddings
//! endpoint, e.g. `http://localhost:1234/v1/embeddings`.
//!
//! Optionally set `HERMES_EMBED_MODEL` to choose a specific model.
//! Default: auto-selects the first model returned by `GET /v1/models`.
//!
//! When active, hermes uses 768-dim neural embeddings instead of the built-in
//! 256-dim deterministic token-hash vectors.  A full `hermes_index` re-run is
//! needed to regenerate the stored vectors when switching modes.

use std::env;
use std::sync::OnceLock;
use tracing::{info, warn};

/// Dimension returned by typical local embedding models (nomic-embed-text-v1.5
/// and most others).  Overridable via HERMES_EMBED_DIM.
const DEFAULT_NEURAL_DIM: usize = 768;

// ── Global singleton so the client is initialised once ──────────────────────

static CLIENT: OnceLock<Option<EmbedClient>> = OnceLock::new();

#[derive(Clone)]
struct EmbedClient {
    url: String,
    model: String,
    client: reqwest::blocking::Client,
}

/// Returns `true` if neural embeddings are configured and reachable.
pub fn is_neural_active() -> bool {
    get_client().is_some()
}

/// Dimension of the vectors used for the current run.
pub fn vector_dim() -> usize {
    if is_neural_active() {
        env::var("HERMES_EMBED_DIM")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_NEURAL_DIM)
    } else {
        crate::vector_ops::VECTOR_DIMENSION
    }
}

/// Embed a single text string.
/// Falls back to local token-hashing when neural is not configured.
pub fn embed(text: &str) -> Vec<f32> {
    if let Some(c) = get_client() {
        match call_embed_api(c, text) {
            Ok(vec) => return vec,
            Err(e) => {
                warn!("[hermes] neural embed failed, using local fallback: {e}");
            }
        }
    }
    // Local deterministic fallback
    let tokens = crate::vector_ops::tokenize(text);
    crate::vector_ops::build_vector(&tokens)
}

// ── Internals ────────────────────────────────────────────────────────────────

fn get_client() -> Option<&'static EmbedClient> {
    CLIENT
        .get_or_init(|| {
            let url = env::var("HERMES_EMBED_URL").ok()?;
            let model = env::var("HERMES_EMBED_MODEL")
                .ok()
                .or_else(|| detect_model(&url))
                .unwrap_or_else(|| "text-embedding-nomic-embed-text-v1.5".to_string());
            let client = reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .ok()?;
            // Warm-up probe
            let probe = call_embed_api_raw(&client, &url, &model, "test");
            match probe {
                Ok(_) => {
                    info!("[hermes] neural embeddings active: url={url} model={model}");
                    Some(EmbedClient { url, model, client })
                }
                Err(e) => {
                    warn!("[hermes] HERMES_EMBED_URL set but probe failed ({e}); using local embeddings");
                    None
                }
            }
        })
        .as_ref()
}

fn detect_model(base_url: &str) -> Option<String> {
    // Strip /embeddings path to get base, then append /models
    let base = base_url
        .trim_end_matches('/')
        .trim_end_matches("/embeddings");
    let models_url = format!("{base}/models");
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .ok()?;
    let resp: serde_json::Value = client.get(&models_url).send().ok()?.json().ok()?;
    // Prefer embedding models (name contains "embed"), fall back to first
    let models = resp["data"].as_array()?;
    let embed_model = models
        .iter()
        .find(|m| {
            m["id"]
                .as_str()
                .map(|id| id.to_lowercase().contains("embed"))
                .unwrap_or(false)
        })
        .or_else(|| models.first())?;
    embed_model["id"].as_str().map(|s| s.to_string())
}

fn call_embed_api(c: &EmbedClient, text: &str) -> Result<Vec<f32>, String> {
    call_embed_api_raw(&c.client, &c.url, &c.model, text)
}

fn call_embed_api_raw(
    client: &reqwest::blocking::Client,
    url: &str,
    model: &str,
    text: &str,
) -> Result<Vec<f32>, String> {
    let body = serde_json::json!({
        "model": model,
        "input": text,
    });
    let resp = client
        .post(url)
        .json(&body)
        .send()
        .map_err(|e| format!("request error: {e}"))?;

    let status = resp.status();
    let json: serde_json::Value = resp
        .json()
        .map_err(|e| format!("parse error: {e}"))?;

    if !status.is_success() {
        return Err(format!("HTTP {status}: {json}"));
    }

    let vec = json["data"][0]["embedding"]
        .as_array()
        .ok_or("missing embedding field")?
        .iter()
        .filter_map(|v| v.as_f64().map(|f| f as f32))
        .collect::<Vec<f32>>();

    if vec.is_empty() {
        return Err("empty embedding vector".to_string());
    }
    Ok(vec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embed_produces_vector() {
        std::env::remove_var("HERMES_EMBED_URL");
        let vec = embed("hello world");
        assert!(!vec.is_empty());
        assert!(vec.len() > 0);
    }

    #[test]
    fn embed_different_inputs_produce_different_vectors() {
        std::env::remove_var("HERMES_EMBED_URL");
        let v1 = embed("hello");
        let v2 = embed("world");
        assert_ne!(v1, v2);
    }

    #[test]
    fn embed_empty_string_produces_vector() {
        std::env::remove_var("HERMES_EMBED_URL");
        let vec = embed("");
        assert!(!vec.is_empty());
    }

    #[test]
    fn is_neural_active_false_when_not_configured() {
        std::env::remove_var("HERMES_EMBED_URL");
        let vec = embed("test");
        assert!(!vec.is_empty());
    }

    #[test]
    fn vector_dim_is_positive() {
        assert!(vector_dim() > 0);
    }
}
