// TRACK-053: Intent-based tool routing and profile-based tool filtering.
//
// Profile filtering (HERMES_TOOL_PROFILE):
//   core     →  8 tools  — local copilot / lightweight agents
//   standard → 16 tools  — CI agents and batch workers
//   full     → all tools — primary Mastermind session (default)
//
// Intent routing (HERMES_TOOL_ROUTING=1):
//   Falls through to full list until per-intent subsets are defined.

use serde_json::Value;

// ---------------------------------------------------------------------------
// Core profile — essential tools every local agent needs
// ---------------------------------------------------------------------------
const CORE_TOOLS: &[&str] = &[
    "hermes_search",       // find code / docs in the knowledge graph
    "hermes_fetch",        // read a specific node returned by search
    "hermes_recall",       // recall prior work before implementation (mandatory)
    "hermes_remember",     // persist session summary at end of conversation
    "hermes_repo_map",     // compact architecture overview within a token budget
    "hermes_index",        // re-index after file changes
    "hermes_validate_symbols", // catch typos in function/struct names before coding
    "hermes_constraints",  // recite layer rules + size budget before generating code
    "hermes_mcp_status",   // diagnostic: node counts, index state — used by ccterm status panel
    "hermes_stats",        // diagnostic: token savings — used by ccterm status panel
];

// ---------------------------------------------------------------------------
// Standard profile — adds governance and memory tools for CI / batch agents
// ---------------------------------------------------------------------------
const STANDARD_TOOLS: &[&str] = &[
    // -- core --
    "hermes_search",
    "hermes_fetch",
    "hermes_recall",
    "hermes_remember",
    "hermes_repo_map",
    "hermes_index",
    "hermes_validate_symbols",
    "hermes_constraints",
    "hermes_mcp_status",
    "hermes_stats",
    // -- standard additions --
    "hermes_write_decision",       // persist resolved decisions
    "hermes_compact_session",      // produce handover artifact
    "hermes_validate_env",         // validate env var names
    "hermes_check_consistency",    // detect Unknown/Unused env vars
    "hermes_impact_analysis",      // blast-radius before refactoring
    "hermes_prepare_commit_message", // structured commit trailers
    "hermes_lint_architecture",    // arch-layer violation scan
    "hermes_quality_baseline",     // snapshot violation set as drift baseline
    "hermes_quality_drift",        // compare current violations vs baseline
    "hermes_fact",                 // record persistent facts
    "hermes_fact_expire",          // expire a superseded fact
    "hermes_facts",                // list active facts
    "hermes_mission_start",        // start a new mission
    "hermes_mission_update",       // transition mission status
    "hermes_mission_event",        // append mission log event
    "hermes_mission_status",       // get mission state
    "hermes_mission_list",         // list missions
];

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Return the `tools/list` payload for a named profile.
/// Unknown profile names fall through to the full list (same as "full").
pub fn tools_payload_for_profile(profile: &str) -> Value {
    let allowed: &[&str] = match profile {
        "core" => CORE_TOOLS,
        "standard" => STANDARD_TOOLS,
        _ => return crate::mcp_tool_schemas::tools_list(), // "full" or unknown
    };
    filter_tools(crate::mcp_tool_schemas::tools_list(), allowed)
}

/// Return the MCP `tools/list` payload appropriate for the given intent string.
/// Falls through to the complete tool list for any unrecognised intent.
pub fn tools_payload_for_intent(intent: &str) -> Value {
    match intent {
        // Future: return filtered subsets for "search", "memory", "quality", etc.
        _ => crate::mcp_tool_schemas::tools_list(),
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn filter_tools(mut payload: Value, allowed: &[&str]) -> Value {
    if let Some(tools) = payload["tools"].as_array_mut() {
        let filtered: Vec<Value> = std::mem::take(tools)
            .into_iter()
            .filter(|t| t["name"].as_str().map_or(false, |n| allowed.contains(&n)))
            .collect();
        payload["tools"] = serde_json::Value::Array(filtered);
    }
    payload
}
