// TRACK-053: ToolActor integration tests.

use crate::mcp_actor::ToolActor;
use crate::HermesEngine;
use serde_json::json;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

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

    fn set(key: &'static str, value: &'static str) -> Self {
        let old = std::env::var(key).ok();
        std::env::set_var(key, value);
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
fn actor_rejects_unknown_tool() {
    let _guard = env_lock().lock().expect("env lock");
    let _profile = EnvVarGuard::clear("HERMES_TOOL_PROFILE");
    let engine = HermesEngine::in_memory("actor-unknown").expect("engine");
    let project_root = tempfile::tempdir().expect("tempdir");
    let actor = ToolActor::start(engine, project_root.path().to_path_buf());

    let err = actor
        .call_tool("hermes_not_real", &json!({}))
        .expect_err("unknown tool should fail")
        .to_string();

    assert!(err.contains("unknown tool"));
}

#[test]
fn actor_accepts_topic_alias_for_recall() {
    let _guard = env_lock().lock().expect("env lock");
    let _timeout = EnvVarGuard::clear("HERMES_TOOL_TIMEOUT_MS");
    let _delay = EnvVarGuard::clear("HERMES_TEST_TOOL_DELAY_MS");
    let engine = HermesEngine::in_memory("actor-recall-alias").expect("engine");
    let project_root = tempfile::tempdir().expect("tempdir");
    let actor = ToolActor::start(engine, project_root.path().to_path_buf());

    let output = actor
        .call_tool("hermes_recall", &json!({ "topic": "track-053" }))
        .expect("recall should accept topic alias");
    let payload: serde_json::Value = serde_json::from_str(&output).expect("valid json");

    assert_eq!(payload["query"], "track-053");
    assert!(payload.get("has_prior_work").is_some());
}

#[test]
fn actor_keeps_tool_calls_responsive_while_auto_index_runs() {
    let _guard = env_lock().lock().expect("env lock");
    let _timeout = EnvVarGuard::clear("HERMES_TOOL_TIMEOUT_MS");
    let _delay = EnvVarGuard::set("HERMES_TEST_INDEX_DELAY_MS", "500");
    let engine = HermesEngine::in_memory("actor-auto-index").expect("engine");
    let project_root = tempfile::tempdir().expect("tempdir");
    std::fs::write(project_root.path().join("main.rs"), "fn main() {}\n").expect("seed file");

    let actor = ToolActor::start(engine, project_root.path().to_path_buf());
    actor.enqueue_auto_index().expect("enqueue auto-index");

    let started = Instant::now();
    // hermes_stats is a read-only tool, so it should use diagnostic_db/read_db
    let output = actor
        .call_tool("hermes_stats", &json!({}))
        .expect("stats call should remain responsive");
    let elapsed = started.elapsed();

    let payload: serde_json::Value = serde_json::from_str(&output).expect("valid json");
    assert!(payload.get("session").is_some());
    assert!(
        elapsed < Duration::from_millis(500),
        "expected responsive stats call while indexing, elapsed={elapsed:?}"
    );
}

#[test]
fn actor_keeps_followup_calls_responsive_after_timeout() {
    let _guard = env_lock().lock().expect("env lock");
    let _timeout = EnvVarGuard::set("HERMES_TOOL_TIMEOUT_MS", "50");
    let _delay = EnvVarGuard::set("HERMES_TEST_TOOL_DELAY_MS", "250");
    let engine = HermesEngine::in_memory("actor-timeout-followup").expect("engine");
    let project_root = tempfile::tempdir().expect("tempdir");
    let actor = ToolActor::start(engine, project_root.path().to_path_buf());

    let err = actor
        .call_tool("hermes_stats", &json!({}))
        .expect_err("first call should time out");
    assert!(err.to_string().contains("tool_timeout"));

    // Remove the artificial delay AND the tight timeout so subsequent calls succeed.
    std::env::remove_var("HERMES_TEST_TOOL_DELAY_MS");
    std::env::remove_var("HERMES_TOOL_TIMEOUT_MS");

    let started = Instant::now();
    let output = actor
        .call_tool("hermes_stats", &json!({}))
        .expect("follow-up call should succeed");
    let elapsed = started.elapsed();

    let payload: serde_json::Value = serde_json::from_str(&output).expect("valid json");
    assert!(payload.get("session").is_some());
    assert!(
        elapsed < Duration::from_millis(200),
        "expected responsive follow-up call after timeout, elapsed={elapsed:?}"
    );
}

#[test]
fn actor_index_accepts_custom_project_root() {
    let _guard = env_lock().lock().expect("env lock");
    let _openai_key = EnvVarGuard::clear("OPENAI_API_KEY");
    let _gemini_key = EnvVarGuard::clear("GEMINI_API_KEY");
    let _lmstudio_url = EnvVarGuard::set("LMSTUDIO_EMBED_URL", "http://127.0.0.1:9999");
    let _timeout = EnvVarGuard::clear("HERMES_TOOL_TIMEOUT_MS");
    let _delay = EnvVarGuard::clear("HERMES_TEST_TOOL_DELAY_MS");
    let engine = HermesEngine::in_memory("actor-index-custom").expect("engine");
    let default_root = tempfile::tempdir().expect("tempdir");
    let custom_root = tempfile::tempdir().expect("tempdir");

    std::fs::write(custom_root.path().join("main.rs"), "fn main() {}\n").expect("seed file");

    let actor = ToolActor::start(engine, default_root.path().to_path_buf());

    let output = actor
        .call_tool(
            "hermes_index",
            &json!({ "project_root": custom_root.path().to_str().unwrap() }),
        )
        .expect("index should accept custom project_root");
    let payload: serde_json::Value = serde_json::from_str(&output).expect("valid json");

    assert_eq!(payload["total_files"], 1);
}
