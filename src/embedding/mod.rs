// tools/hermes-engine/src/embedding/mod.rs
pub mod generator;
mod provider_types;
mod routing;

#[cfg(test)]
mod generator_tests;

pub use generator::EmbeddingGenerator;
use std::env;

pub fn embedding_mode() -> &'static str {
    if env_has("HERMES_LLM_GATEWAY_URL") {
        return "llm-gateway";
    }
    if env_has("LMSTUDIO_URL") || env_has("LMSTUDIO_EMBED_URL") {
        return "lmstudio";
    }
    if env_has("OPENAI_API_KEY") || env_has("GEMINI_API_KEY") {
        return "lmstudio (default local/aostar) -> model fallback";
    }
    "lmstudio"
}

fn env_has(key: &str) -> bool {
    env::var(key).map(|k| !k.is_empty()).unwrap_or(false)
}

pub fn embed_text(text: &str) -> Vec<f32> {
    if tokio::runtime::Handle::try_current().is_ok() {
        let owned = text.to_string();
        if let Ok(Some(vec)) = std::thread::spawn(move || try_embed_text(&owned)).join() {
            return vec;
        }
    } else if let Some(vec) = try_embed_text(text) {
        return vec;
    }
    deterministic_embedding(text, EmbeddingGenerator::dimension())
}

fn try_embed_text(text: &str) -> Option<Vec<f32>> {
    if let Ok(gen) = EmbeddingGenerator::new() {
        if let Ok(rt) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            if let Ok(vec) = rt.block_on(gen.generate_embedding(text)) {
                return Some(vec);
            }
        }
    }
    None
}

pub(crate) fn deterministic_embedding(text: &str, dim: usize) -> Vec<f32> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    let hash = hasher.finalize();
    let mut emb = Vec::with_capacity(dim);
    for i in 0..dim {
        emb.push((hash[i % hash.len()] as f32) / 255.0);
    }
    emb
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dimension_is_1024() {
        assert_eq!(EmbeddingGenerator::dimension(), 1024);
    }

    #[test]
    fn deterministic_embedding_is_repeatable() {
        let a = deterministic_embedding("foo", 10);
        let b = deterministic_embedding("foo", 10);
        assert_eq!(a, b);
    }
}
