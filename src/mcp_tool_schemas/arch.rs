use serde_json::{json, Value};

pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "hermes_lint_architecture",
            "description": "TRACK-045: Scan the knowledge graph for architecture violations: layer boundary breaches (LAYER-001..005), file/method size limits (SIZE-001..002), safety anti-patterns (SAFETY-001..003), concurrency issues (CONCURRENCY-001), and query injection risks (QUERY-001). Defaults to mode='summary' (counts + worst-N per rule) to keep tool output small for LLM callers; use mode='iterative' with rule_id to drill into a specific rule, or mode='full' for the legacy unbounded violation list. When `scope` is omitted and mode != 'full', the scan is auto-restricted to git-changed files (vs HEAD + untracked) so single-bug fixes do not pay for a whole-repo scan; pass auto_scope=false to force a full scan.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "mode":           { "type": "string",  "description": "Output mode: 'summary' (default, counts + top-N per rule), 'iterative' (one rule at a time, requires rule_id), 'full' (every violation — can be very large)", "enum": ["summary", "iterative", "full"] },
                    "rule_id":        { "type": "string",  "description": "Required when mode='iterative'. Single rule ID to drill into (e.g. 'LAYER-001'). Discover IDs via mode='summary' first." },
                    "max_violations": { "type": "integer", "description": "Cap on violations returned in iterative mode (default 20)" },
                    "worst_per_rule": { "type": "integer", "description": "Top-N violations to surface per rule in summary mode (default 5)" },
                    "scope":          { "type": "string",  "description": "Optional file path, directory, or crate name to limit scope. When omitted and mode!='full', auto-derived from git-changed files." },
                    "auto_scope":     { "type": "boolean", "description": "When true (default) and scope is omitted in summary/iterative mode, restrict the scan to git-changed files vs HEAD plus untracked. Set false to force a whole-repo scan." },
                    "severity_min":   { "type": "string",  "description": "Minimum severity to report: 'error' | 'warning' | 'info' (default: 'warning')", "enum": ["error", "warning", "info"] },
                    "rules":          { "type": "array",   "items": { "type": "string" },                         "description": "Specific rule IDs to check (e.g. ['LAYER-001','SIZE-001']). Default: all 26 rules." }
                }
            }
        }),
        json!({
            "name": "hermes_heal_violations",
            "description": "TRACK-048: Generate constrained healing candidates from hermes_lint_architecture output. Phase 1 is model-locked to gpt-5-mini and only auto-selects SAFETY-001/002/003 candidates.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "scope":        { "type": "string",  "description": "Optional path/crate scope passed through to lint" },
                    "severity_min": { "type": "string",  "description": "Optional severity floor for lint: error|warning|info", "enum": ["error", "warning", "info"] },
                    "rules":        { "type": "array",   "items": { "type": "string" }, "description": "Optional lint rule filter" },
                    "dry_run":      { "type": "boolean", "description": "When true (default), returns candidates without applying edits" },
                    "max_items":    { "type": "integer", "description": "Maximum candidate count to return (default 25)" },
                    "model":        { "type": "string",  "description": "Must be 'gpt-5-mini' in TRACK-048" }
                }
            }
        }),
        json!({
            "name": "hermes_constraints",
            "description": "TRACK-045: Return the architectural constraints applicable to a specific file and optional line range. Call this BEFORE generating code at any location to get: layer classification, applicable rules with rationale, naming convention, size budget (lines remaining before 300-line limit), and available patterns. Implements 'recitation before generation'.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file_path":   { "type": "string",  "description": "Target file path (relative to project root or absolute)" },
                    "line_start":  { "type": "integer", "description": "Optional start line for scoped constraint query" },
                    "line_end":    { "type": "integer", "description": "Optional end line for scoped constraint query" }
                },
                "required": ["file_path"]
            }
        }),
        json!({
            "name": "hermes_test_coverage_map",
            "description": "TRACK-045: Map test→implementation edges via the knowledge graph. Returns covered symbols (with their test names and files), uncovered symbols (no test edge), and an overall coverage ratio (0.0–1.0). Coverage is inferred from Tests edges built from file naming conventions and import analysis — no test runner required.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "symbol": { "type": "string", "description": "Specific symbol name to inspect (default: all)" },
                    "scope":  { "type": "string", "description": "File or directory path scope filter (default: all)" }
                }
            }
        }),
        json!({
            "name": "hermes_impact_analysis",
            "description": "Graph-Based Impact Analysis: Traces the knowledge graph to find all downstream symbols and files that depend on a specific symbol. Helps determine the 'blast radius' of a change.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "symbol_name": { "type": "string", "description": "The name of the symbol to analyze (e.g. 'User', 'fetch_data')" }
                },
                "required": ["symbol_name"]
            }
        }),
        json!({
            "name": "hermes_scan_duplicates",
            "description": "Scan a function/struct signature for semantically similar symbols using vector embeddings. Returns matches with similarity scores.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "signature": { "type": "string", "description": "The function/struct signature or preview text to scan" }
                },
                "required": ["signature"]
            }
        }),
        json!({
            "name": "hermes_prepare_commit_message",
            "description": "Generate a commit message body with structured trailers (Task-Model, Decision-Doc, Session-Note, Docs, Pipeline) so SRE build healing has traceable context.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "subject":      { "type": "string", "description": "Conventional commit subject line" },
                    "body":         { "type": "string", "description": "Optional commit body text" },
                    "task_model":   { "type": "string", "description": "Task model URI or identifier" },
                    "decision_doc": { "type": "string", "description": "Path to decision doc" },
                    "session_note": { "type": "string", "description": "Path to session note/checkpoint" },
                    "docs":         { "type": "array", "items": { "type": "string" }, "description": "Docs paths to reference" },
                    "pipelines":    { "type": "array", "items": { "type": "integer" }, "description": "Explicit pipeline IDs; if omitted, inferred from changes" },
                    "changes":      { "type": "array", "items": { "type": "string" }, "description": "Changed file paths for pipeline inference" }
                },
                "required": ["subject"]
            }
        }),
        json!({
            "name": "hermes_repo_map",
            "description": "Generate a token-budget-constrained repository map. Returns a compact listing of all code symbols (functions, structs, traits, enums) ranked by reference count (xref edges). Use to give the LLM a global architecture overview without reading entire files.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "max_tokens": {
                        "type": "integer",
                        "description": "Maximum token budget for the map output (default: 2048)",
                        "default": 2048
                    }
                }
            }
        }),
        json!({
            "name": "hermes_validate_env",
            "description": "Validate an environment variable name against the config registry. Returns whether it's known and suggests similar names if not found.",
            "inputSchema": {
                "type": "object",
                "properties": { "env_var": { "type": "string", "description": "Environment variable name to validate" } },
                "required": ["env_var"]
            }
        }),
        json!({
            "name": "hermes_validate_symbols",
            "description": "Validate symbol names (functions, structs, traits, etc.) against the knowledge graph. Returns whether each symbol exists and suggests the closest known names for any that are not found.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "symbols": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "List of symbol names to validate (e.g. [\"ingest_file\", \"XrefExtractor\"])"
                    }
                },
                "required": ["symbols"]
            }
        }),
        json!({
            "name": "hermes_search_misses",
            "description": "Post-mortem report of hermes_search calls that returned zero results. Use this to understand what topics the knowledge graph cannot answer — repeated misses indicate indexing gaps or indexing exclusions that should be reviewed.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "since_days": {
                        "type": "integer",
                        "description": "Restrict to misses recorded in the last N days. Omit for all-time view."
                    },
                    "top_k": {
                        "type": "integer",
                        "description": "How many top repeated missed queries to surface in the aggregation (default: 10)."
                    }
                }
            }
        }),
        json!({
            "name": "hermes_match_skills",
            "description": "Search the indexed skill library for reusable workflows matching a task query. Returns ranked matches with path, description, scope, and category. Skills are indexed from SKILL.md assets in the project.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Natural-language task query (e.g. 'cosmos db migration', 'http request builder')" },
                    "scope": { "type": "string", "description": "Optional scope filter: 'project', 'shared', or omit for all" }
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "hermes_fetch_skill",
            "description": "Fetch the full content of a skill by file path or skill ID. Returns the complete SKILL.md instructions plus any declared resource roots (scripts/, references/, assets/).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "skill_path": { "type": "string", "description": "File path or skill ID to fetch" }
                },
                "required": ["skill_path"]
            }
        }),
        json!({
            "name": "hermes_check_consistency",
            "description": "Active Guardian: Scans the knowledge graph for environment variable inconsistencies. Reports variables used in code but not defined in .env/docs (Unknown), and variables defined but never used (Unused).",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "hermes_log_incident",
            "description": "Open a new incident for a sub-product. Creates a structured incident file under memory/incidents/<sub_product>/. Use when a production issue, pipeline failure, or service degradation is detected. Sub-products: backend, frontend, daemon, trainer, telegram-gateway, llm-gateway, doctor, watchdog, codex-worker, android, infra, hermes, ccterm.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "sub_product": { "type": "string", "description": "Affected sub-product (e.g. 'backend', 'telegram-gateway')" },
                    "title":       { "type": "string", "description": "Short incident title (e.g. 'Cosmos auth timeout under load')" },
                    "severity":    { "type": "string", "description": "P0 (critical/down), P1 (major degradation), P2 (partial impact), P3 (minor)", "enum": ["P0", "P1", "P2", "P3"] },
                    "symptoms":    { "type": "string", "description": "Observable symptoms and error messages" },
                    "tags":        { "type": "array", "items": { "type": "string" }, "description": "Classification tags" }
                },
                "required": ["sub_product", "title"]
            }
        }),
        json!({
            "name": "hermes_resolve_incident",
            "description": "Resolve an open incident. Updates the incident file to RESOLVED status and automatically writes a KB article unless write_kb is false. Call this after the fix is confirmed green in CI.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "sub_product":    { "type": "string", "description": "Sub-product the incident belongs to" },
                    "slug":           { "type": "string", "description": "Incident slug returned by hermes_log_incident or visible in memory/incidents/" },
                    "root_cause":     { "type": "string", "description": "Root cause explanation" },
                    "fix_summary":    { "type": "string", "description": "What was done to fix the issue" },
                    "files_changed":  { "type": "array", "items": { "type": "string" }, "description": "Files modified as part of the fix" },
                    "lessons":        { "type": "string", "description": "Lessons learned / prevention advice" },
                    "write_kb":       { "type": "boolean", "description": "Auto-write KB article (default: true)" }
                },
                "required": ["sub_product", "slug"]
            }
        }),
        json!({
            "name": "hermes_query_incidents",
            "description": "List incidents from the incident ledger, with optional filters. Use at session start to surface open incidents for the current sub-product.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "sub_product": { "type": "string", "description": "Filter by sub-product (omit for all)" },
                    "status":      { "type": "string", "description": "Filter by status: OPEN or RESOLVED" },
                    "severity":    { "type": "string", "description": "Filter by severity: P0, P1, P2, P3" }
                }
            }
        }),
        json!({
            "name": "hermes_write_kb_article",
            "description": "Write a standalone Knowledge Base article to memory/kb/<sub_product>/. Use for recurring issues, architectural gotchas, or fixes that should be remembered across teams.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "sub_product":        { "type": "string", "description": "Sub-product this article applies to" },
                    "title":              { "type": "string", "description": "KB article title" },
                    "problem":            { "type": "string", "description": "Description of the problem / symptoms" },
                    "root_cause":         { "type": "string", "description": "Root cause" },
                    "solution":           { "type": "string", "description": "Fix / solution steps" },
                    "prevention":         { "type": "string", "description": "How to prevent this in future" },
                    "related_incidents":  { "type": "array", "items": { "type": "string" }, "description": "Related incident slugs" },
                    "tags":               { "type": "array", "items": { "type": "string" }, "description": "Classification tags" },
                    "slug":               { "type": "string", "description": "Optional filename slug (auto-derived from title if omitted)" }
                },
                "required": ["title"]
            }
        }),
        json!({
            "name": "hermes_search_kb",
            "description": "Search the Knowledge Base articles in memory/kb/. Returns ranked KB articles matching the query. Optionally scoped to a single sub-product.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query":       { "type": "string", "description": "Search query (e.g. 'cosmos auth timeout', 'OOM daemon')" },
                    "sub_product": { "type": "string", "description": "Optional sub-product scope filter" }
                },
                "required": ["query"]
            }
        }),
    ]
}
