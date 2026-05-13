use serde_json::{json, Value};

fn project_root_schema() -> Value {
    json!({
        "type": "string",
        "description": "Project name (e.g. 'lonaspark') or absolute path. Use hermes_list_projects to see available projects. Wrong value → wrong repo's results."
    })
}

pub(crate) fn handle_tools_list() -> Value {
    json!({
        "tools": [
            {
                "name": "hermes_list_projects",
                "description": "List all projects known to this hermes server (default + HERMES_PROJECTS registry + previously accessed). Use the returned project_root values for all other hermes tools.",
                "inputSchema": {
                    "type": "object",
                    "properties": {}
                }
            },
            {
                "name": "hermes_search",
                "description": "Search the codebase knowledge graph. Returns pointers (not full content). Records token savings in accounting.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Natural-language or keyword search query" },
                        "project_root": project_root_schema()
                    },
                    "required": ["query"]
                }
            },
            {
                "name": "hermes_fetch",
                "description": "Fetch full content for a specific knowledge-graph node by ID returned by hermes_search.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "node_id": { "type": "string", "description": "Node ID from a previous search result" },
                        "project_root": project_root_schema()
                    },
                    "required": ["node_id"]
                }
            },
            {
                "name": "hermes_index",
                "description": "Re-index the project files into the knowledge graph. Run after adding or changing files.",
                "inputSchema": {
                    "type": "object",
                    "properties": { "project_root": project_root_schema() },
                    "required": []
                }
            },
            {
                "name": "hermes_stats",
                "description": "Return cumulative token savings statistics across all Hermes sessions.",
                "inputSchema": {
                    "type": "object",
                    "properties": { "project_root": project_root_schema() },
                    "required": []
                }
            },
            {
                "name": "hermes_fact",
                "description": "Record a persistent fact (decision, learning, constraint, etc.) into the temporal store.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "fact_type": { "type": "string", "description": "One of: architecture, decision, learning, constraint, error_pattern, api_contract" },
                        "content":   { "type": "string", "description": "The fact to record" },
                        "project_root": project_root_schema()
                    },
                    "required": ["fact_type", "content"]
                }
            },
            {
                "name": "hermes_facts",
                "description": "List active facts from the temporal store, optionally filtered by type.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "fact_type": { "type": "string", "description": "Optional filter type (omit for all)" },
                        "project_root": project_root_schema()
                    },
                    "required": []
                }
            },
            {
                "name": "hermes_validate_env",
                "description": "Validate an environment variable name against the config_registry populated during hermes_index. Returns valid:true when the name is known, or valid:false with up to 5 Levenshtein-closest suggestions.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "env_var": { "type": "string", "description": "The environment variable name to validate (e.g. DATABASE_URL)" },
                        "project_root": project_root_schema()
                    },
                    "required": ["env_var"]
                }
            },
            {
                "name": "hermes_check_consistency",
                "description": "Scan config_registry for env vars that are used in code but not defined (unknown) or defined but never referenced (unused). Run after hermes_index.",
                "inputSchema": {
                    "type": "object",
                    "properties": { "project_root": project_root_schema() },
                    "required": []
                }
            },
            {
                "name": "hermes_mcp_status",
                "description": "Return current server status: indexing state, total node/file counts, and capability flags. Requires no arguments.",
                "inputSchema": {
                    "type": "object",
                    "properties": {}
                }
            }
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_all_ten_tools() {
        let result = handle_tools_list();
        let tools = result["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 10);
    }

    #[test]
    fn tools_have_required_fields() {
        let result = handle_tools_list();
        let tools = result["tools"].as_array().unwrap();
        for tool in tools {
            assert!(tool["name"].is_string(), "tool missing name");
            assert!(tool["description"].is_string(), "tool missing description");
            assert!(tool["inputSchema"].is_object(), "tool missing inputSchema");
        }
    }

    #[test]
    fn hermes_search_requires_query() {
        let result = handle_tools_list();
        let search_tool = result["tools"].as_array().unwrap()
            .iter().find(|t| t["name"] == "hermes_search").unwrap();
        let required = search_tool["inputSchema"]["required"].as_array().unwrap();
        assert!(required.iter().any(|r| r == "query"));
    }

    #[test]
    fn hermes_fetch_requires_node_id() {
        let result = handle_tools_list();
        let fetch_tool = result["tools"].as_array().unwrap()
            .iter().find(|t| t["name"] == "hermes_fetch").unwrap();
        let required = fetch_tool["inputSchema"]["required"].as_array().unwrap();
        assert!(required.iter().any(|r| r == "node_id"));
    }
}
