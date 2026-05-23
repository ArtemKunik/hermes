use serde_json::{json, Value};

pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "hermes_quality_review",
            "description": "Run an LLM-driven quality review of source files under a given path. Reviews each file against up to 14 architectural dimensions (QD-01..QD-14). Returns a summary of new findings added to .hermes/quality-state.json.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Relative path to review (e.g. 'ChartApp/chartapp-server-rust/src'). Defaults to 'ChartApp'."
                    },
                    "dim": {
                        "type": "string",
                        "description": "Optional single dimension filter (e.g. 'QD-01'). Omit to run all 14 dimensions."
                    },
                    "tier": {
                        "type": "string",
                        "description": "Optional minimum tier filter ('T1'|'T2'|'T3'|'T4'). Only run dimensions at or above this tier."
                    }
                }
            }
        }),
        json!({
            "name": "hermes_quality_score",
            "description": "Return the quality score table per crate/module and the overall project score. Scores range 0-100 (100 = no open findings).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "module": {
                        "type": "string",
                        "description": "Optional crate/module name to filter results."
                    },
                    "trend": {
                        "type": "boolean",
                        "description": "If true, include score delta vs previous scan."
                    }
                }
            }
        }),
        json!({
            "name": "hermes_quality_next",
            "description": "Return the single highest-priority open finding across the project (or a specific module). Priority = tier_weight x zone_multiplier.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "module": {
                        "type": "string",
                        "description": "Optional crate/module name to restrict the search."
                    }
                }
            }
        }),
        json!({
            "name": "hermes_quality_resolve",
            "description": "Mark a finding as resolved. The finding's score penalty is removed and module/project scores are recomputed.",
            "inputSchema": {
                "type": "object",
                "required": ["id"],
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "Finding ID (e.g. 'Q-A1B2C3D4')."
                    }
                }
            }
        }),
        json!({
            "name": "hermes_quality_baseline",
            "description": "Snapshot the current arch-lint violation set as the drift baseline. Subsequent calls to hermes_quality_drift compare against this snapshot to identify new violations.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "hermes_quality_drift",
            "description": "Compare current arch-lint violations against the stored baseline. Returns new violations (regressions), fixed violations (improvements), per-rule deltas, and an overall trend ('improving'|'stable'|'degrading').",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "hermes_quality_wontfix",
            "description": "Mark a finding as won't-fix with a mandatory reason. The penalty is halved (not removed).",
            "inputSchema": {
                "type": "object",
                "required": ["id", "reason"],
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "Finding ID (e.g. 'Q-A1B2C3D4')."
                    },
                    "reason": {
                        "type": "string",
                        "description": "Mandatory explanation of why this finding is acceptable."
                    }
                }
            }
        }),
        json!({
            "name": "hermes_quality_dismiss",
            "description": "Dismiss a quality finding. It disappears from active lists but remains in the database to avoid resurfacing.",
            "inputSchema": {
                "type": "object",
                "required": ["id"],
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "Finding ID (e.g. 'Q-A1B2C3D4')."
                    },
                    "reason": {
                        "type": "string",
                        "description": "Optional reason for dismissal."
                    }
                }
            }
        }),
        json!({
            "name": "hermes_lint_dismiss",
            "description": "Dismiss a lint violation or skill candidate by ID. Stored in DB so it won't resurface in future scans.",
            "inputSchema": {
                "type": "object",
                "required": ["item_type", "item_id"],
                "properties": {
                    "item_type": {
                        "type": "string",
                        "enum": ["violation", "skill_candidate"],
                        "description": "Type of item to dismiss."
                    },
                    "item_id": {
                        "type": "string",
                        "description": "Unique identifier (fingerprint for violations, name for skill candidates)."
                    },
                    "reason": {
                        "type": "string",
                        "description": "Optional reason for dismissal."
                    }
                }
            }
        }),
        json!({
            "name": "hermes_dismissed_list",
            "description": "List all dismissed items (findings, violations, skill candidates) for the current project.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "item_type": {
                        "type": "string",
                        "enum": ["finding", "violation", "skill_candidate"],
                        "description": "Optional filter by item type."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Max results to return. Defaults to 100."
                    }
                }
            }
        }),
        json!({
            "name": "hermes_auto_dismiss",
            "description": "Auto-dismiss open quality findings older than N days (default 30). Useful for cleaning up stale items.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "max_age_days": {
                        "type": "integer",
                        "description": "Max age in days before auto-dismiss. Defaults to 30."
                    }
                }
            }
        }),
    ]
}
