// TRACK-053: MCP server integration tests.

use serde_json::json;
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
fn tools_list_returns_all_tools_when_routing_disabled() {
    let _guard = env_lock().lock().expect("env lock");
    let _profile = EnvVarGuard::set("HERMES_TOOL_PROFILE", "full");
    let _routing = EnvVarGuard::clear("HERMES_TOOL_ROUTING");

    let payload = crate::mcp_server::handle_tools_list(&json!({ "intent": "memory" }));
    let default_payload = crate::mcp_tool_schemas::tools_list();

    let payload_tools = payload["tools"].as_array().expect("tools array");
    let default_tools = default_payload["tools"].as_array().expect("tools array");

    assert_eq!(payload_tools.len(), default_tools.len());
}

#[test]
fn tools_list_filters_by_intent_when_routing_enabled() {
    let _guard = env_lock().lock().expect("env lock");
    let _profile = EnvVarGuard::set("HERMES_TOOL_PROFILE", "full");
    let _routing = EnvVarGuard::set("HERMES_TOOL_ROUTING", "1");

    let payload = crate::mcp_server::handle_tools_list(&json!({ "intent": "memory" }));
    let routed = crate::tool_router::tools_payload_for_intent("memory");

    assert_eq!(payload, routed);

    let tools = payload["tools"].as_array().expect("tools array");
    let has_recall = tools
        .iter()
        .any(|tool| tool["name"].as_str() == Some("hermes_recall"));
    assert!(has_recall, "memory routing should include hermes_recall");
}
