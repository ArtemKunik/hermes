use serde_json::{json, Value};

pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "hermes_slow_loop_status",
            "description": "Return the current status of the Hermes Slow Loop (digests, compaction, skill candidates).",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "hermes_generate_digest",
            "description": "Manually trigger a daily digest generation for a specific date (YYYY-MM-DD).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "date": { "type": "string", "description": "Target date in YYYY-MM-DD format" }
                },
                "required": ["date"]
            }
        }),
        json!({
            "name": "hermes_compact_sessions",
            "description": "Manually trigger session compaction (archive sessions older than 14 days).",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "hermes_generate_weekly_brief",
            "description": "Manually trigger weekly pattern detection and skill candidate generation based on recent daily digests.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "hermes_approve_skill_candidate",
            "description": "Approve a candidate skill and promote it to the formal skill library.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Candidate skill name (from memory/slow_loop/skill_candidates/)" }
                },
                "required": ["name"]
            }
        }),
        json!({
            "name": "hermes_reject_skill_candidate",
            "description": "Reject a candidate skill and archive it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Candidate skill name" }
                },
                "required": ["name"]
            }
        }),
        json!({
            "name": "hermes_apply_proposal",
            "description": "Apply a drift correction proposal to the codebase.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "filename": { "type": "string", "description": "Proposal filename (from memory/slow_loop/proposals/)" }
                },
                "required": ["filename"]
            }
        }),
        json!({
            "name": "hermes_list_tracks",
            "description": "List conductor tracks with normalized status, progress, and next-step hints.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "status": {
                        "type": "string",
                        "description": "Filter by status: unfinished, active, in-progress, planned, speccing, blocked, completed, all"
                    }
                }
            }
        }),
        json!({
            "name": "hermes_resume_track",
            "description": "Prepare a continuation brief for an unfinished conductor track without changing code.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "track_id": { "type": "string", "description": "Track id like TRACK-062" },
                    "auto": { "type": "boolean", "description": "Automatically pick the best unfinished track" },
                    "status": { "type": "string", "description": "When auto=true, limit selection to a status bucket like active or unfinished" }
                }
            }
        }),
    ]
}
