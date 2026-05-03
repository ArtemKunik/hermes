use serde_json::{json, Value};

pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "hermes_fact",
            "description": "Record a persistent fact into the temporal store. Facts are atomic, typed knowledge claims with lifecycle metadata. 'fact_type' defaults to 'observation' when omitted.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "content":          { "type": "string", "description": "The fact text. Must be non-empty." },
                    "fact_type":        { "type": "string", "description": "One of: decision, constraint, assumption, observation, dependency, learning, architecture, api_contract, error_pattern. Defaults to 'observation'." },
                    "topic":            { "type": "string", "description": "Free-text grouping label (e.g. 'cosmos-auth')" },
                    "tags":             { "type": "array", "items": { "type": "string" }, "description": "Classification tags" },
                    "confidence":       { "type": "number", "minimum": 0, "maximum": 1, "description": "Confidence score in [0.0, 1.0]" },
                    "ttl":              { "type": "string", "description": "ISO 8601 duration string (e.g. 'P7D' = 7 days, 'PT1H' = 1 hour). Sets valid_to automatically." },
                    "source_reference": { "type": "string", "description": "Human-readable reference to the source (file path, URL, document name)" },
                    "provenance":       { "type": "string", "description": "Structured provenance object serialised as JSON: { source_kind, path, hash }" },
                    "repo_id":          { "type": "string", "description": "Repository scope for the fact" },
                    "agent_id":         { "type": "string", "description": "Agent that recorded the fact" }
                },
                "required": ["content"]
            }
        }),
        json!({
            "name": "hermes_facts",
            "description": "List facts from the temporal store. Active facts only by default; use include_expired=true to include stale ones.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "fact_type":      { "type": "string", "description": "Filter by type (omit for all)" },
                    "topic":          { "type": "string", "description": "Filter by topic label" },
                    "tags":           { "type": "array", "items": { "type": "string" }, "description": "Filter to facts containing ALL specified tags" },
                    "repo_id":        { "type": "string", "description": "Scope to a repository" },
                    "limit":          { "type": "integer", "description": "Maximum number of results (default: 50)" },
                    "include_expired":{ "type": "boolean", "description": "If true, also return expired facts annotated with stale: true" }
                }
            }
        }),
        json!({
            "name": "hermes_fact_expire",
            "description": "Expire (soft-delete) an active fact, optionally linking the replacement. Use when a fact is superseded by a newer claim.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "fact_id":      { "type": "string", "description": "ID of the fact to expire" },
                    "superseded_by":{ "type": "string", "description": "Optional ID of the fact that replaces this one" }
                },
                "required": ["fact_id"]
            }
        }),
        json!({
            "name": "hermes_remember",
            "description": "Save a conversation session summary to memory. Creates a structured markdown file in memory/sessions/ that Hermes auto-indexes, enabling recall of past decisions and context in future sessions.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "topic":         { "type": "string",  "description": "Session topic / title (e.g. 'Cosmos DB partitioning')" },
                    "summary":       { "type": "string",  "description": "Concise summary of the conversation and outcomes" },
                    "tags":          { "type": "array",   "items": { "type": "string" }, "description": "Classification tags (e.g. ['arch-decision', 'cosmos-db'])" },
                    "files_touched": { "type": "array",   "items": { "type": "string" }, "description": "Files modified during this session" },
                    "decisions":     { "type": "array",   "items": { "type": "string" }, "description": "Key decisions made with rationale" },
                    "problems":      { "type": "array",   "items": { "type": "string" }, "description": "Problems encountered and how they were resolved" },
                    "actions":       { "type": "array",   "items": { "type": "string" }, "description": "Remaining action items / follow-ups" }
                },
                "required": ["topic", "summary"]
            }
        }),
        json!({
            "name": "hermes_compact_session",
            "description": "Compact active session context into a continuation-focused artifact. Returns a summary, a structured handover markdown body, relevant files, and next actions. Optionally persists the handover under memory/handover/ and indexes it immediately.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "topic": { "type": "string", "description": "Primary task/topic for the handover. The alias `task` is also accepted." },
                    "task": { "type": "string", "description": "Alias for `topic`." },
                    "summary": { "type": "string", "description": "Current concise status before compaction." },
                    "recent_messages": { "type": "array", "items": { "type": "string" }, "description": "Recent message or step snippets to preserve as context." },
                    "accomplished": { "type": "array", "items": { "type": "string" }, "description": "Completed steps or summarized progress items." },
                    "files_touched": { "type": "array", "items": { "type": "string" }, "description": "Files that matter for continuation." },
                    "decisions": { "type": "array", "items": { "type": "string" }, "description": "Important decisions that should not be rediscovered." },
                    "problems": { "type": "array", "items": { "type": "string" }, "description": "Current problems, risks, or blockers." },
                    "actions": { "type": "array", "items": { "type": "string" }, "description": "Concrete next actions for the next session." },
                    "active_constraints": { "type": "array", "items": { "type": "string" }, "description": "Architectural or process constraints still in effect." },
                    "recent_errors": { "type": "array", "items": { "type": "string" }, "description": "Recent errors worth preserving in the handover." },
                    "continuation_prompt": { "type": "string", "description": "Optional ready-to-run continuation prompt for the next session." },
                    "target_token_budget": { "type": "integer", "description": "Approximate budget used to keep the summary compact.", "default": 1200 },
                    "persist_handover": { "type": "boolean", "description": "When true, write the handover to memory/handover/ and index it immediately." }
                },
                "required": []
            }
        }),
        json!({
            "name": "hermes_recall",
            "description": "Recall prior work on a topic. Searches memory for related decisions and sessions, returns a structured context brief with what was tried, what worked/failed, and recommended next steps. Accepts `query` and also `topic` as an alias. Use BEFORE starting implementation to avoid repeating past dead ends.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Topic or problem to recall prior work for (e.g. 'android keyboard webview')" },
                    "topic": { "type": "string", "description": "Alias for query. Accepted for compatibility with older clients." }
                },
                "required": []
            }
        }),
        json!({
            "name": "hermes_write_decision",
            "description": "Create or update a decision document in memory/decisions/. Use immediately when a non-trivial problem is RESOLVED — records Status, Context, What Was Tried, What Didn't Work, Root Cause, Next Steps, Related Files, and Tags. The Tags section drives future hermes_recall hit rate.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title":         { "type": "string", "description": "Decision title, e.g. 'Android keyboard overlap'" },
                    "status":        { "type": "string", "description": "OPEN | PARTIALLY RESOLVED | RESOLVED | ABANDONED" },
                    "context":       { "type": "string", "description": "1-3 sentences: what problem, which component, why it matters" },
                    "what_worked":   { "type": "array",  "items": { "type": "string" }, "description": "Approaches that worked, each as a string describing the method" },
                    "what_failed":   { "type": "array",  "items": { "type": "string" }, "description": "Dead-end approaches and why they failed" },
                    "root_cause":    { "type": "string", "description": "Best current understanding of root cause" },
                    "next_steps":    { "type": "array",  "items": { "type": "string" }, "description": "Concrete untried next actions" },
                    "related_files": { "type": "array",  "items": { "type": "string" }, "description": "Relevant source file paths" },
                    "tags":          { "type": "array",  "items": { "type": "string" }, "description": "3-6 lowercase kebab-case tags: component, error-pattern, fix-type" },
                    "slug":          { "type": "string", "description": "Optional filename slug (auto-derived from title if omitted)" }
                },
                "required": ["title"]
            }
        }),
        json!({
            "name": "hermes_memory_stats",
            "description": "Return memory usage statistics: sessions saved, search recall hits, memory hit rate. Shows how often past conversation memory is being recalled.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "hermes_battery_check",
            "description": "Determine if a periodic battery‑change review is due (defaults 10 sessions / 7 days).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "session_threshold": { "type": "number", "description": "Number of sessions that trigger review" },
                    "day_threshold": { "type": "number", "description": "Days since last review that trigger review" }
                }
            }
        }),
        json!({
            "name": "hermes_query_memory",
            "description": "Search the Qdrant semantic_memory collection for chunks relevant to a query. Embeds the query and returns the top-N matching memory chunks with file_path, section_heading, text, and similarity score. Use this to ground responses in past project knowledge before starting implementation.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Natural-language query to search for in semantic memory" },
                    "limit": { "type": "number", "description": "Maximum number of chunks to return (default: 5)" }
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "hermes_get_core_facts",
            "description": "Read and return the contents of memory/CORE_FACTS.md — the durable project knowledge base maintained by the Philosopher. Contains service topology, infrastructure details, key env var conventions, and architectural decisions.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
    ]
}
