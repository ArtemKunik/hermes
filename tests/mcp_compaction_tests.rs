use hermes_engine::mcp_compaction::tool_compact_session;
use hermes_engine::{mcp_tools, HermesEngine};
use serde_json::json;

#[test]
fn compact_session_requires_topic_or_task() {
    let dir = tempfile::tempdir().unwrap();
    let engine = HermesEngine::in_memory("compact-missing-topic").unwrap();

    let err = tool_compact_session(&engine, dir.path(), &json!({})).unwrap_err();

    assert!(err.to_string().contains("topic"));
}

#[test]
fn compact_session_returns_minimal_artifact_without_persistence() {
    let dir = tempfile::tempdir().unwrap();
    let engine = HermesEngine::in_memory("compact-minimal").unwrap();

    let response = tool_compact_session(
        &engine,
        dir.path(),
        &json!({
            "task": "Investigate MCP lifecycle"
        }),
    )
    .unwrap();
    let payload: serde_json::Value = serde_json::from_str(&response).unwrap();

    assert_eq!(payload["persisted"], false);
    assert_eq!(payload["indexed"], false);
    assert!(payload["summary"]
        .as_str()
        .unwrap()
        .contains("Investigate MCP lifecycle"));
    assert!(payload["handover"]
        .as_str()
        .unwrap()
        .contains("# Handover: Investigate MCP lifecycle"));
}

#[test]
fn compact_session_persists_rich_handover_and_indexes_it() {
    let dir = tempfile::tempdir().unwrap();
    let engine = HermesEngine::in_memory("compact-rich").unwrap();

    let response = tool_compact_session(
        &engine,
        dir.path(),
        &json!({
            "topic": "TRACK-041 Phase 1 compaction",
            "summary": "Implemented the first continuation flow for Hermes.",
            "recent_messages": [
                "Identified the Phase 1 scope from TRACK-041.",
                "Chose a new module instead of extending mcp_memory.rs."
            ],
            "accomplished": [
                "Defined the compaction payload contract.",
                "Added tests for minimal and persisted handovers."
            ],
            "files_touched": [
                "tools/hermes-engine/src/mcp_compaction.rs",
                "tools/hermes-engine/src/mcp_actor.rs",
                "tools/hermes-engine/src/mcp_compaction.rs"
            ],
            "decisions": [
                "Keep compaction deterministic and continuation-oriented."
            ],
            "problems": [
                "Hermes MCP tools were unavailable in this session."
            ],
            "actions": [
                "Wire hermes_compact_session into the MCP actor.",
                "Update autosave docs for caller responsibilities."
            ],
            "active_constraints": [
                "Do not grow mcp_memory.rs further."
            ],
            "recent_errors": [
                "No Hermes MCP tools were attached to this chat runtime."
            ],
            "continuation_prompt": "Resume Phase 1 wiring, then run Hermes tests.",
            "persist_handover": true,
            "target_token_budget": 600
        }),
    )
    .unwrap();
    let payload: serde_json::Value = serde_json::from_str(&response).unwrap();
    let handover_path = dir.path().join(payload["handover_path"].as_str().unwrap());

    assert_eq!(payload["persisted"], true);
    assert_eq!(payload["indexed"], true);
    assert_eq!(payload["relevant_files"].as_array().unwrap().len(), 2);
    assert_eq!(payload["next_actions"].as_array().unwrap().len(), 2);
    assert!(handover_path.exists());

    let handover = std::fs::read_to_string(&handover_path).unwrap();
    assert!(handover.contains("## Active Task State"));
    assert!(handover.contains("Resume Phase 1 wiring"));
    assert!(handover.contains("Implemented the first continuation flow for Hermes."));

    let search = mcp_tools::tool_search(&engine, "continuation flow", None).unwrap();
    let search_json: serde_json::Value = serde_json::from_str(&search).unwrap();
    let pointers = search_json["pointers"].as_array().unwrap();
    assert!(!pointers.is_empty());
}
