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
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project_root": { "type": "string", "description": "Optional repository root path to index" },
                    "repo_root": { "type": "string", "description": "Optional repository root path to index (alias for project_root)" }
                }
            }
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
        json!({
            "name": "hermes_lookup",
            "description": "O(1) symbol lookup: find where a function, struct, class, or type is defined. Returns file path, line number, kind, exported flag, and methods (for impl blocks).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "symbol_name": { "type": "string", "description": "Symbol name to find (e.g. 'verify_token', 'AuthService')" }
                },
                "required": ["symbol_name"]
            }
        }),
        json!({
            "name": "hermes_file_symbols",
            "description": "List all symbols defined in a file with their line numbers, kinds, and exported status. Returns an array of symbol entries sorted by line.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file_path": { "type": "string", "description": "File path relative to project root (e.g. 'src/auth.rs')" }
                },
                "required": ["file_path"]
            }
        }),
        json!({
            "name": "hermes_graph",
            "description": "Retrieve a subgraph of nodes and edges from the knowledge graph. Optionally filter by node IDs, node types, and edge types. Returns both nodes and edges.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_ids": { "type": "array", "items": { "type": "string" }, "description": "Specific node IDs to include (omit for all)" },
                    "node_types": { "type": "array", "items": { "type": "string" }, "description": "Filter by node types (e.g. ['function', 'struct', 'file'])" },
                    "edge_types": { "type": "array", "items": { "type": "string" }, "description": "Filter by edge types (e.g. ['calls', 'imports'])" },
                    "limit": { "type": "integer", "description": "Maximum number of nodes/edges to return (default: 100)" }
                }
            }
        }),
        json!({
            "name": "hermes_neighbors",
            "description": "Get 1-hop neighbors of a node (both incoming and outgoing edges). Returns connected nodes with edge details and direction.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "string", "description": "ID of the node to get neighbors for" },
                    "edge_types": { "type": "array", "items": { "type": "string" }, "description": "Optional filter by edge types (e.g. ['calls', 'imports'])" },
                    "limit": { "type": "integer", "description": "Maximum number of neighbors to return (default: 50)" }
                },
                "required": ["node_id"]
            }
        }),
    ]
}
