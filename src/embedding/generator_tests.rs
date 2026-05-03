use super::embedding_mode;
use super::generator::EmbeddingGenerator;
use std::sync::{Mutex, OnceLock};

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn lock_env() -> std::sync::MutexGuard<'static, ()> {
    env_lock().lock().unwrap_or_else(|e| e.into_inner())
}

fn snapshot_env(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

fn restore_env(key: &str, value: Option<String>) {
    match value {
        Some(current) => std::env::set_var(key, current),
        None => std::env::remove_var(key),
    }
}

#[test]
fn test_new_uses_lmstudio_embed_model_when_present_expected() {
    let _guard = lock_env();
    let provider_model = snapshot_env("GEMINI_EMBEDDING_MODEL");
    let embed_model = snapshot_env("LMSTUDIO_EMBED_MODEL");
    let chat_model = snapshot_env("LMSTUDIO_MODEL");

    std::env::set_var("GEMINI_EMBEDDING_MODEL", "text-embedding-004");
    std::env::set_var("LMSTUDIO_MODEL", "chat-model");
    std::env::set_var("LMSTUDIO_EMBED_MODEL", "embed-model");

    let generator = EmbeddingGenerator::new().expect("generator should build");
    assert_eq!(generator.lmstudio_model(), "embed-model");

    restore_env("GEMINI_EMBEDDING_MODEL", provider_model);
    restore_env("LMSTUDIO_EMBED_MODEL", embed_model);
    restore_env("LMSTUDIO_MODEL", chat_model);
}

#[test]
fn test_new_uses_lmstudio_model_when_embed_model_missing_expected() {
    let _guard = lock_env();
    let provider_model = snapshot_env("GEMINI_EMBEDDING_MODEL");
    let embed_model = snapshot_env("LMSTUDIO_EMBED_MODEL");
    let chat_model = snapshot_env("LMSTUDIO_MODEL");

    std::env::set_var("GEMINI_EMBEDDING_MODEL", "text-embedding-004");
    std::env::remove_var("LMSTUDIO_EMBED_MODEL");
    std::env::set_var("LMSTUDIO_MODEL", "chat-model");

    let generator = EmbeddingGenerator::new().expect("generator should build");
    assert_eq!(generator.lmstudio_model(), "chat-model");

    restore_env("GEMINI_EMBEDDING_MODEL", provider_model);
    restore_env("LMSTUDIO_EMBED_MODEL", embed_model);
    restore_env("LMSTUDIO_MODEL", chat_model);
}

#[test]
fn test_new_uses_aostar_embed_model_when_env_missing_expected() {
    let _guard = lock_env();
    let computer_name = snapshot_env("COMPUTERNAME");
    let hostname = snapshot_env("HOSTNAME");
    let provider_model = snapshot_env("GEMINI_EMBEDDING_MODEL");
    let embed_model = snapshot_env("LMSTUDIO_EMBED_MODEL");
    let chat_model = snapshot_env("LMSTUDIO_MODEL");

    std::env::set_var("COMPUTERNAME", "AOSTAR-MACO");
    std::env::remove_var("HOSTNAME");
    std::env::set_var("GEMINI_EMBEDDING_MODEL", "text-embedding-004");
    std::env::remove_var("LMSTUDIO_EMBED_MODEL");
    std::env::remove_var("LMSTUDIO_MODEL");

    let generator = EmbeddingGenerator::new().expect("generator should build");
    assert_eq!(generator.lmstudio_model(), "qwen3-embedding");

    restore_env("COMPUTERNAME", computer_name);
    restore_env("HOSTNAME", hostname);
    restore_env("GEMINI_EMBEDDING_MODEL", provider_model);
    restore_env("LMSTUDIO_EMBED_MODEL", embed_model);
    restore_env("LMSTUDIO_MODEL", chat_model);
}

#[test]
fn test_new_uses_legion_local_lmstudio_before_aostar_when_env_missing_expected() {
    let _guard = lock_env();
    let computer_name = snapshot_env("COMPUTERNAME");
    let hostname = snapshot_env("HOSTNAME");
    let lmstudio_url = snapshot_env("LMSTUDIO_URL");
    let embed_url = snapshot_env("LMSTUDIO_EMBED_URL");

    std::env::set_var("COMPUTERNAME", "ARTEM-LEGION");
    std::env::remove_var("HOSTNAME");
    std::env::remove_var("LMSTUDIO_URL");
    std::env::remove_var("LMSTUDIO_EMBED_URL");

    let generator = EmbeddingGenerator::new().expect("generator should build");
    assert_eq!(
        generator.lmstudio_targets(),
        vec![
            (
                "http://127.0.0.1:1234",
                "nomic-ai/text-embedding-nomic-embed-text-v1.5"
            ),
            ("http://aostar-maco:8001", "qwen3-embedding"),
        ]
    );

    restore_env("COMPUTERNAME", computer_name);
    restore_env("HOSTNAME", hostname);
    restore_env("LMSTUDIO_URL", lmstudio_url);
    restore_env("LMSTUDIO_EMBED_URL", embed_url);
}

#[test]
fn test_new_uses_aostar_lmstudio_when_not_legion_and_env_missing_expected() {
    let _guard = lock_env();
    let computer_name = snapshot_env("COMPUTERNAME");
    let hostname = snapshot_env("HOSTNAME");
    let lmstudio_url = snapshot_env("LMSTUDIO_URL");
    let embed_url = snapshot_env("LMSTUDIO_EMBED_URL");

    std::env::set_var("COMPUTERNAME", "AOSTAR-MACO");
    std::env::remove_var("HOSTNAME");
    std::env::remove_var("LMSTUDIO_URL");
    std::env::remove_var("LMSTUDIO_EMBED_URL");

    let generator = EmbeddingGenerator::new().expect("generator should build");
    assert_eq!(
        generator.lmstudio_targets(),
        vec![("http://aostar-maco:8001", "qwen3-embedding")]
    );

    restore_env("COMPUTERNAME", computer_name);
    restore_env("HOSTNAME", hostname);
    restore_env("LMSTUDIO_URL", lmstudio_url);
    restore_env("LMSTUDIO_EMBED_URL", embed_url);
}

#[test]
fn test_new_uses_explicit_lmstudio_url_without_default_fallbacks_expected() {
    let _guard = lock_env();
    let computer_name = snapshot_env("COMPUTERNAME");
    let hostname = snapshot_env("HOSTNAME");
    let lmstudio_url = snapshot_env("LMSTUDIO_URL");
    let embed_url = snapshot_env("LMSTUDIO_EMBED_URL");

    std::env::set_var("COMPUTERNAME", "ARTEM-LEGION");
    std::env::remove_var("HOSTNAME");
    std::env::set_var("LMSTUDIO_URL", "http://custom-lmstudio:1234");
    std::env::remove_var("LMSTUDIO_EMBED_URL");

    let generator = EmbeddingGenerator::new().expect("generator should build");
    assert_eq!(
        generator.lmstudio_targets(),
        vec![("http://custom-lmstudio:1234", "qwen3-embedding")]
    );

    restore_env("COMPUTERNAME", computer_name);
    restore_env("HOSTNAME", hostname);
    restore_env("LMSTUDIO_URL", lmstudio_url);
    restore_env("LMSTUDIO_EMBED_URL", embed_url);
}

#[test]
fn test_embedding_mode_uses_aostar_embeddings_when_env_missing_expected() {
    let _guard = lock_env();
    let openai_key = snapshot_env("OPENAI_API_KEY");
    let gemini_key = snapshot_env("GEMINI_API_KEY");
    let gateway_url = snapshot_env("HERMES_LLM_GATEWAY_URL");
    let lmstudio_url = snapshot_env("LMSTUDIO_URL");
    let embed_url = snapshot_env("LMSTUDIO_EMBED_URL");

    std::env::remove_var("OPENAI_API_KEY");
    std::env::remove_var("GEMINI_API_KEY");
    std::env::remove_var("HERMES_LLM_GATEWAY_URL");
    std::env::remove_var("LMSTUDIO_URL");
    std::env::remove_var("LMSTUDIO_EMBED_URL");

    assert_eq!(embedding_mode(), "lmstudio");

    restore_env("OPENAI_API_KEY", openai_key);
    restore_env("GEMINI_API_KEY", gemini_key);
    restore_env("HERMES_LLM_GATEWAY_URL", gateway_url);
    restore_env("LMSTUDIO_URL", lmstudio_url);
    restore_env("LMSTUDIO_EMBED_URL", embed_url);
}

#[test]
fn test_embedding_mode_reports_local_aostar_fallback_when_provider_key_present_expected() {
    let _guard = lock_env();
    let openai_key = snapshot_env("OPENAI_API_KEY");
    let gemini_key = snapshot_env("GEMINI_API_KEY");
    let gateway_url = snapshot_env("HERMES_LLM_GATEWAY_URL");
    let lmstudio_url = snapshot_env("LMSTUDIO_URL");
    let embed_url = snapshot_env("LMSTUDIO_EMBED_URL");

    std::env::remove_var("OPENAI_API_KEY");
    std::env::set_var("GEMINI_API_KEY", "dummy-key");
    std::env::remove_var("HERMES_LLM_GATEWAY_URL");
    std::env::remove_var("LMSTUDIO_URL");
    std::env::remove_var("LMSTUDIO_EMBED_URL");

    assert_eq!(
        embedding_mode(),
        "lmstudio (default local/aostar) -> model fallback"
    );

    restore_env("OPENAI_API_KEY", openai_key);
    restore_env("GEMINI_API_KEY", gemini_key);
    restore_env("HERMES_LLM_GATEWAY_URL", gateway_url);
    restore_env("LMSTUDIO_URL", lmstudio_url);
    restore_env("LMSTUDIO_EMBED_URL", embed_url);
}
