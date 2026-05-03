// tools/hermes-engine/src/mcp_constraints.rs
// TRACK-045: MCP tool handler for hermes_constraints.

use anyhow::Result;
use rusqlite::params;
use serde_json::Value;
use std::path::Path;

use crate::arch_rules::classifier::{classify_file, naming_convention, Layer};
use crate::arch_rules::default_rules;
use crate::graph::KnowledgeGraph;
use crate::HermesEngine;

/// MCP tool: hermes_constraints
///
/// Returns the applicable architecture rules for a given file/line range.
/// Called by the agent BEFORE generating code at that location.
///
/// Parameters:
///   file_path  (string)  — target file path
///   line_start (number?) — optional start line
///   line_end   (number?) — optional end line
pub fn tool_constraints(
    engine: &HermesEngine,
    project_root: &Path,
    args: &Value,
) -> Result<String> {
    let file_path = args["file_path"].as_str().unwrap_or("");
    let line_start = args["line_start"].as_u64();
    let line_end = args["line_end"].as_u64();

    let layer = classify_file(file_path);
    let layer_str = layer.as_str();

    // Derive crate/package from path
    let crate_or_package = infer_crate(file_path);

    // Applicable rules for this layer
    let applicable_rules = rules_for_layer(&layer);

    // Current file line count (from graph) for size budget
    let graph = KnowledgeGraph::new(engine.read_db().clone(), engine.project_id());
    let file_lines = get_file_lines(&graph, file_path).unwrap_or(0);
    let lines_remaining = 300_i64.saturating_sub(file_lines);

    // Available patterns matching layer keywords
    let patterns_available = find_patterns(project_root, &layer);

    let output = serde_json::json!({
        "file_path": file_path,
        "layer": layer_str,
        "crate_or_package": crate_or_package,
        "line_range": {
            "start": line_start,
            "end": line_end,
        },
        "applicable_rules": applicable_rules,
        "naming_convention": naming_convention(&layer),
        "size_budget": {
            "file_lines_current": file_lines,
            "file_lines_remaining": lines_remaining,
            "method_line_limit": 50,
        },
        "patterns_available": patterns_available,
    });
    Ok(serde_json::to_string_pretty(&output)?)
}

fn infer_crate(file_path: &str) -> String {
    let norm = file_path.replace('\\', "/");
    // Walk path segments to find known crate roots
    let crate_names = [
        "chartapp-server-rust",
        "hermes-engine",
        "llm-gateway-rust",
        "telegram-gateway-rust",
        "trainer-worker-rust",
        "doctor-service-rust",
        "watchdog-rust",
        "codex-worker-rust",
        "mastermind-daemon-rust",
        "chartapp.client",
        "android-app",
    ];
    for name in &crate_names {
        if norm.contains(name) {
            return name.to_string();
        }
    }
    "unknown".to_string()
}

fn rules_for_layer(layer: &Layer) -> Vec<serde_json::Value> {
    let all = default_rules();
    let relevant_ids: &[&str] = match layer {
        Layer::Handler => &["LAYER-001", "LAYER-002", "SIZE-001", "SIZE-002", "SAFETY-001", "SAFETY-002"],
        Layer::Service => &["LAYER-003", "SIZE-001", "SIZE-002", "SAFETY-001", "SAFETY-002", "CONCURRENCY-001"],
        Layer::Store => &["QUERY-001", "SIZE-001", "SIZE-002", "SAFETY-001", "SAFETY-002"],
        Layer::Component => &["LAYER-004", "LAYER-005", "SIZE-001", "SIZE-002", "SAFETY-003"],
        Layer::Hook => &["SIZE-001", "SIZE-002", "SAFETY-003"],
        Layer::Api => &["SIZE-001", "SIZE-002", "SAFETY-003"],
        Layer::Type => &["SAFETY-003", "SIZE-001"],
        Layer::Test => &[],
        Layer::Unknown => &["SIZE-001", "SIZE-002"],
    };

    all.iter()
        .filter(|r| relevant_ids.contains(&r.id()))
        .map(|r| serde_json::json!({
            "rule_id": r.id(),
            "severity": r.severity().as_str(),
            "description": r.description(),
            "applies_because": applies_because(r.id(), layer),
        }))
        .collect()
}

fn applies_because(rule_id: &str, layer: &Layer) -> &'static str {
    match (rule_id, layer) {
        ("LAYER-001", _) => "File is in handlers/ — must not import store modules directly",
        ("LAYER-002", _) => "File is in handlers/ — handler functions must stay ≤30 lines",
        ("LAYER-003", _) => "File is in *_service/ — services must not import handlers",
        ("LAYER-004", _) => "File is in components/ — components must not call fetch/axios directly",
        ("LAYER-005", _) => "File is in components/ — use hooks/services for API calls",
        ("QUERY-001", _) => "File is in store module — always use parameterized queries",
        ("SIZE-001", _) => "All source files must stay ≤300 lines (AGENTS.md hard limit)",
        ("SIZE-002", _) => "All methods must stay ≤50 lines (AGENTS.md hard limit)",
        ("SAFETY-001", _) => "Production Rust must not panic via unwrap()",
        ("SAFETY-002", _) => "Production Rust must not panic via expect()",
        ("SAFETY-003", _) => "TypeScript `any` requires a // SAFETY: justification comment",
        ("CONCURRENCY-001", _) => "Async Rust must use Arc<T> not Rc for thread safety",
        _ => "Applies to this file's architectural layer",
    }
}

fn get_file_lines(graph: &KnowledgeGraph, file_path: &str) -> Option<i64> {
    let conn = graph.db().lock().ok()?;
    conn.query_row(
        "SELECT end_line - start_line FROM nodes
         WHERE project_id = ?1 AND file_path = ?2 AND node_type = 'file'
         LIMIT 1",
        params![graph.project_id(), file_path],
        |row| row.get(0),
    )
    .ok()
}

fn find_patterns(project_root: &Path, layer: &Layer) -> Vec<String> {
    let patterns_dir = project_root.join("patterns");
    if !patterns_dir.exists() { return vec![]; }

    let keywords: &[&str] = match layer {
        Layer::Handler => &["handler", "route", "actix"],
        Layer::Service => &["service", "business"],
        Layer::Store => &["store", "cosmos", "query", "db"],
        Layer::Component => &["component", "react"],
        Layer::Hook => &["hook", "use"],
        Layer::Api => &["api", "http", "client"],
        _ => &[],
    };

    let Ok(entries) = std::fs::read_dir(&patterns_dir) else { return vec![] };
    entries
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| {
            let lower = name.to_lowercase();
            keywords.iter().any(|kw| lower.contains(kw))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn constraints_returns_valid_json() {
        let engine = crate::HermesEngine::in_memory("constraints-test").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let result = tool_constraints(
            &engine,
            dir.path(),
            &json!({ "file_path": "src/handlers/task_handler.rs" }),
        )
        .unwrap();
        let v: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["layer"], "handler");
    }

    #[test]
    fn constraints_classifies_store() {
        let engine = crate::HermesEngine::in_memory("constraints-store").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let result = tool_constraints(
            &engine,
            dir.path(),
            &json!({ "file_path": "src/store_cosmos/tasks.rs" }),
        )
        .unwrap();
        let v: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["layer"], "store");
        assert!(v["applicable_rules"].as_array().unwrap()
            .iter().any(|r| r["rule_id"] == "QUERY-001"));
    }
}
