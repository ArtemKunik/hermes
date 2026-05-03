use serde_json::{json, Value};

pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "hermes_mission_start",
            "description": "Start a new mission in preflight status. Appends a phase_enter log entry. mission_id is globally unique.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title":       { "type": "string", "description": "Mission title" },
                    "description": { "type": "string", "description": "Mission description" },
                    "tags":        { "type": "array",  "items": { "type": "string" }, "description": "Classification tags" }
                },
                "required": ["title"]
            }
        }),
        json!({
            "name": "hermes_mission_update",
            "description": "Transition mission status and/or update metadata. Status transitions are validated against the state machine (preflight→active→landing→completed/abandoned). Auto-recall runs on active; auto-review triggers on landing.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "mission_id":   { "type": "string", "description": "Mission ID" },
                    "status":       { "type": "string", "description": "Target status: preflight, active, landing, completed, abandoned" },
                    "title":        { "type": "string", "description": "Updated title" },
                    "description":  { "type": "string", "description": "Updated description" },
                    "tags":         { "type": "string", "description": "Updated tags (comma-separated)" },
                    "checklist":    { "type": "string", "description": "Updated checklist (JSON array)" },
                    "diff":         { "type": "string", "description": "Diff content for review" },
                    "commit_range": { "type": "string", "description": "Commit range for review (e.g. abc123..def456)" },
                    "event_type":   { "type": "string", "description": "Optional custom event to append to the log" },
                    "event_data":   { "type": "object", "description": "Data payload for the custom event" }
                },
                "required": ["mission_id"]
            }
        }),
        json!({
            "name": "hermes_mission_list",
            "description": "List missions, optionally filtered by status. Returns newest-updated first.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "status": { "type": "string", "description": "Filter by status (omit for all)" },
                    "limit":  { "type": "number", "description": "Max results (default: 20)" }
                }
            }
        }),
    ]
}
