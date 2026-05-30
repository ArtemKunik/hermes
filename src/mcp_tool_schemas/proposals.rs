use serde_json::{json, Value};

pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "hermes_proposal_create",
            "description": "Batch-create proposals in the ideation store. Each proposal must have a 'title'. Default status is 'pending'. Use for ingesting scanner output.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "proposals": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id":             { "type": "string", "description": "Optional ID (auto-generated if omitted)" },
                                "title":          { "type": "string", "description": "Proposal title" },
                                "description":    { "type": "string", "description": "Detailed description" },
                                "source":         { "type": "string", "description": "Source enum: feature_gap | telemetry | domain_driven | user_request | ideation | arch_linter | github_trending" },
                                "priority":       { "type": "integer", "description": "Priority 1-10 (default: 5)" },
                                "status":         { "type": "string", "description": "Status: pending | approved | rejected | edited (default: pending)" },
                                "evidence_ids":   { "type": "array", "items": { "type": "string" }, "description": "Evidence IDs backing this proposal" },
                                "why_it_matters": { "type": "string", "description": "Why this proposal matters" },
                                "next_step":      { "type": "string", "description": "Recommended next action" },
                                "repo":           { "type": "string", "description": "Repository name" },
                                "fingerprint":    { "type": "string", "description": "Deduplication fingerprint" }
                            },
                            "required": ["title"]
                        }
                    }
                },
                "required": ["proposals"]
            }
        }),
        json!({
            "name": "hermes_proposal_list",
            "description": "List proposals with optional status/source filters. Returns an array of proposal objects.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "status":  { "type": "string", "description": "Filter by status (pending | approved | rejected | edited). Omit for all." },
                    "source":  { "type": "string", "description": "Filter by source (e.g. feature_gap, github_trending). Omit for all." },
                    "limit":   { "type": "integer", "description": "Maximum records to return (default: 50)" }
                }
            }
        }),
        json!({
            "name": "hermes_proposal_update",
            "description": "Update editable fields of a proposal: title, description, priority, next_step, why_it_matters. Only provided fields are updated.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "proposal_id":    { "type": "string", "description": "Proposal ID" },
                    "title":          { "type": "string", "description": "Updated title" },
                    "description":    { "type": "string", "description": "Updated description" },
                    "priority":       { "type": "integer", "description": "Updated priority 1-10" },
                    "next_step":      { "type": "string", "description": "Updated next step" },
                    "why_it_matters": { "type": "string", "description": "Updated rationale" }
                },
                "required": ["proposal_id"]
            }
        }),
        json!({
            "name": "hermes_proposal_reject",
            "description": "Reject a proposal with a reason. Sets status to 'rejected' and records the rejection reason for quality feedback.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "proposal_id": { "type": "string", "description": "Proposal ID to reject" },
                    "reason":      { "type": "string", "description": "Rejection reason (feeds back into quality gate)" }
                },
                "required": ["proposal_id", "reason"]
            }
        }),
        json!({
            "name": "hermes_proposal_approve",
            "description": "Approve a pending/edited proposal. Creates a linked Hermes mission in 'preflight' status and transitions the proposal to 'approved'. Returns both the updated proposal and the new mission.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "proposal_id": { "type": "string", "description": "Proposal ID to approve" }
                },
                "required": ["proposal_id"]
            }
        }),
    ]
}
