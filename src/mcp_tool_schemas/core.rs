use serde_json::{json, Value};

pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "hermes_search",
            "description": "Search the codebase knowledge graph. Returns pointers (not full content). Optionally accepts a 'goal' hint to bias results toward a specific information need (e.g. 'error handling in API gateway').",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Natural-language or keyword search query" },
                    "goal":  { "type": "string", "description": "Optional goal hint — describes the agent's current information need to bias search ranking toward relevant results" }
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "hermes_fetch",
            "description": "Fetch full content for a specific knowledge-graph node by ID returned by hermes_search.",
            "inputSchema": {
                "type": "object",
                "properties": { "node_id": { "type": "string", "description": "Node ID from a previous search result" } },
                "required": ["node_id"]
            }
        }),
        json!({
            "name": "hermes_index",
            "description": "Re-index the project files into the knowledge graph. Run after adding or changing files.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "hermes_backfill",
            "description": "Backfill stored content token counts for existing nodes (retro-fill).",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "hermes_stats",
            "description": "Return cumulative token savings statistics across all Hermes sessions.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "hermes_mcp_status",
            "description": "Return structured MCP runtime health and metadata: node counts, index state, DB config, search cache stats, tool inventory, and capability hints. Use for operator diagnostics and multi-MCP toolchain health checks.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "hermes_tools",
            "description": "Return a routed subset of tool schemas for a given intent. Use this before selecting a Hermes tool when tool routing is enabled to reduce schema overload and surface the 5-7 most relevant tools.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "intent": { "type": "string", "description": "Intent string such as 'recall previous work', 'index project', or 'quality review'." }
                },
                "required": ["intent"]
            }
        }),
    ]
}
