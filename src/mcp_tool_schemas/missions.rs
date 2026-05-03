use serde_json::{json, Value};

pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "hermes_mission_start",
            "description": "Create a new mission in 'preflight' status. A mission is a named, stateful container for multi-step agent work that persists context across sessions.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title":       { "type": "string", "description": "Short mission title (e.g. 'Implement Cosmos auth refactor')" },
                    "description": { "type": "string", "description": "Detailed description of the work" },
                    "tags":        { "type": "array", "items": { "type": "string" }, "description": "Classification tags" },
                    "checklist":   { "type": "array", "items": { "type": "string" }, "description": "Ordered list of deliverable tasks" },
                    "repo_id":     { "type": "string", "description": "Repository scope" },
                    "agent_id":    { "type": "string", "description": "Agent ID creating the mission" }
                },
                "required": ["title"]
            }
        }),
        json!({
            "name": "hermes_mission_update",
            "description": "Transition mission status and/or update metadata. Enforces the state machine: preflight→active→landing→completed; any→aborted; landing→active (re-open). When transitioning to active, auto-recall is triggered. When transitioning to landing, auto-review fires if a reviewer is configured.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "mission_id":  { "type": "string", "description": "Identifier of the existing mission" },
                    "status":      { "type": "string", "description": "Target status: preflight | active | landing | completed | aborted" },
                    "title":       { "type": "string", "description": "Updated title" },
                    "description": { "type": "string", "description": "Updated description" },
                    "tags":        { "type": "array", "items": { "type": "string" }, "description": "Replaced tag list" },
                    "checklist":   { "type": "array", "items": { "type": "string" }, "description": "Replaced checklist" },
                    "diff":        { "type": "string", "description": "Passed to auto-review when transitioning to landing" },
                    "commit_range":{ "type": "string", "description": "Passed to auto-review when transitioning to landing" }
                },
                "required": ["mission_id"]
            }
        }),
        json!({
            "name": "hermes_mission_event",
            "description": "Append a timestamped event to the mission log (append-only). Use to record phase transitions, artifacts produced, decisions made, blockers, or user confirmations.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "mission_id": { "type": "string", "description": "Mission ID" },
                    "event_type": {
                        "type": "string",
                        "description": "Event type: phase_enter | artifact | decision | task_progress | choice | blocked | requirements_confirmed | (any freeform value)"
                    },
                    "data": {
                        "type": "object",
                        "description": "Event-specific payload (e.g. {phase:'execution'} for phase_enter, {kind:'spec',path:'...'} for artifact)"
                    }
                },
                "required": ["mission_id", "event_type"]
            }
        }),
        json!({
            "name": "hermes_mission_status",
            "description": "Retrieve the current state of a single mission including its full event log.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "mission_id": { "type": "string", "description": "Mission ID" }
                },
                "required": ["mission_id"]
            }
        }),
        json!({
            "name": "hermes_mission_list",
            "description": "List missions with optional filters. Returns summary objects (same shape as hermes_mission_status output).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "status":  { "type": "string", "description": "Filter by status value (omit for all)" },
                    "repo_id": { "type": "string", "description": "Filter by repository" },
                    "limit":   { "type": "integer", "description": "Maximum records to return (default: 20)" }
                }
            }
        }),
    ]
}
