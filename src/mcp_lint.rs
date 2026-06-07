// tools/hermes-engine/src/mcp_lint.rs
// TRACK-045: MCP tool handler for hermes_lint_architecture.

use anyhow::Result;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;

use crate::arch_rules::{default_rules, Severity, Violation};
use crate::graph::KnowledgeGraph;
use crate::mcp_lint_scope::{git_changed_files, ResolvedScope};
use crate::HermesEngine;

/// MCP tool: hermes_lint_architecture
///
/// Parameters:
///   mode           (string?)  — "summary" | "iterative" | "full" (default: "summary")
///                               summary: counts + worst-N per rule, no full violations array
///                               iterative: violations for one rule_id, capped at max_violations
///                               full: every violation (legacy behavior, can be very large)
///   rule_id        (string?)  — required when mode="iterative"; single rule ID to drill into
///   max_violations (integer?) — cap for iterative mode (default 20)
///   worst_per_rule (integer?) — top-N violations to show per rule in summary (default 5)
///   scope          (string?)  — file path, directory, or crate name (default: auto from git)
///   auto_scope     (bool?)    — when scope is omitted and mode != "full", derive scope from
///                               git-changed files vs HEAD + untracked. Default true. Set to
///                               false to force a whole-repo scan in summary/iterative mode.
///   severity_min   (string?)  — "error" | "warning" | "info" (default: "warning")
///   rules          (string[]?) — specific rule IDs to check (default: all)
pub fn tool_lint_architecture(
    engine: &HermesEngine,
    project_root: &Path,
    args: &Value,
) -> Result<String> {
    let mode = Mode::parse(args["mode"].as_str().unwrap_or("summary"));
    let iter_rule = args["rule_id"].as_str().map(|s| s.to_string());
    let max_violations = args["max_violations"].as_u64().unwrap_or(20) as usize;
    let worst_per_rule = args["worst_per_rule"].as_u64().unwrap_or(5) as usize;

    if matches!(mode, Mode::Iterative) && iter_rule.is_none() {
        return Err(anyhow::anyhow!(
            "mode=\"iterative\" requires rule_id; call mode=\"summary\" first to discover rule IDs"
        ));
    }

    let scope = args["scope"].as_str();
    let auto_scope = args["auto_scope"].as_bool().unwrap_or(true);
    let resolved_scope = if let Some(s) = scope {
        ResolvedScope::explicit(s)
    } else if auto_scope && !matches!(mode, Mode::Full) {
        let files = git_changed_files(project_root);
        if files.is_empty() {
            ResolvedScope::all()
        } else {
            ResolvedScope::from_git(files)
        }
    } else {
        ResolvedScope::all()
    };
    let severity_min = Severity::parse_str(args["severity_min"].as_str().unwrap_or("warning"));
    let mut rule_filter: Vec<&str> = args["rules"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    // In iterative mode, hard-restrict the rule scan to the requested rule_id.
    if let (Mode::Iterative, Some(id)) = (&mode, iter_rule.as_deref()) {
        rule_filter.clear();
        rule_filter.push(id);
    }

    let graph = KnowledgeGraph::new(engine.read_db().clone(), engine.project_id());
    let rules = default_rules();
    let start = std::time::Instant::now();

    let mut all_violations: Vec<Violation> = Vec::new();
    let mut scanned_nodes: u64 = 0;

    for rule in &rules {
        // Filter by requested rule IDs
        if !rule_filter.is_empty() && !rule_filter.contains(&rule.id()) {
            continue;
        }
        // Skip rules below severity threshold
        if rule.severity() < severity_min {
            continue;
        }

        match rule.evaluate(&graph, project_root) {
            Ok(mut violations) => {
                violations.retain(|v| resolved_scope.matches(&v.file_path));
                all_violations.extend(violations);
            }
            Err(e) => {
                eprintln!("[hermes-lint] rule {} failed: {e}", rule.id());
            }
        }
        scanned_nodes += count_nodes(&graph).unwrap_or(0);
    }

    // Build summary
    let total = all_violations.len();
    let mut by_severity: HashMap<&str, usize> =
        HashMap::from([("error", 0), ("warning", 0), ("info", 0)]);
    let mut by_rule: HashMap<String, usize> = HashMap::new();
    let mut unique_ids = std::collections::HashSet::new();

    for v in &all_violations {
        *by_severity.entry(v.severity.as_str()).or_insert(0) += 1;
        *by_rule.entry(v.rule_id.clone()).or_insert(0) += 1;
        // Unique violation = (rule, file, line)
        let unique_key = format!("{}:{}:{:?}", v.rule_id, v.file_path, v.line_start);
        unique_ids.insert(unique_key);
    }

    let elapsed_ms = start.elapsed().as_millis() as u64;

    // Build worst-N-per-rule slice (used by Summary mode).
    let mut worst_by_rule: HashMap<String, Vec<&Violation>> = HashMap::new();
    for v in &all_violations {
        let entry = worst_by_rule.entry(v.rule_id.clone()).or_default();
        if entry.len() < worst_per_rule {
            entry.push(v);
        }
    }
    let by_rule_detail: Vec<Value> = by_rule
        .iter()
        .map(|(rule_id, count)| {
            let worst: Vec<Value> = worst_by_rule
                .get(rule_id)
                .map(|vs| {
                    vs.iter()
                        .map(|v| {
                            json!({
                                "file": v.file_path,
                                "line": v.line_start,
                                "severity": v.severity.as_str(),
                                "message": v.message,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            json!({ "rule_id": rule_id, "count": count, "worst": worst })
        })
        .collect();

    let summary = json!({
        "total": total,
        "total_unique_violations": unique_ids.len(),
        "by_severity": by_severity,
        "by_rule": by_rule,
        "by_rule_detail": by_rule_detail,
    });

    let output = match mode {
        Mode::Summary => {
            json!({
                "mode": "summary",
                "summary": summary,
                "scope_source": resolved_scope.source,
                "scope_files": resolved_scope.files,
                "scanned_nodes": scanned_nodes,
                "elapsed_ms": elapsed_ms,
                "next": "Call mode=\"iterative\" with rule_id=<id> from by_rule_detail to drill into specific violations.",
            })
        }
        Mode::Iterative => {
            // rule_id is guaranteed Some here (validated above).
            let rule = iter_rule.as_deref().unwrap_or("");
            let mut filtered: Vec<&Violation> = all_violations
                .iter()
                .filter(|v| v.rule_id == rule)
                .collect();
            let total_for_rule = filtered.len();
            filtered.truncate(max_violations);
            let truncated = total_for_rule > max_violations;
            json!({
                "mode": "iterative",
                "rule_id": rule,
                "violations": filtered,
                "total_for_rule": total_for_rule,
                "returned": filtered.len(),
                "truncated": truncated,
                "summary": summary,
                "scope_source": resolved_scope.source,
                "scope_files": resolved_scope.files,
                "scanned_nodes": scanned_nodes,
                "elapsed_ms": elapsed_ms,
            })
        }
        Mode::Full => {
            json!({
                "mode": "full",
                "violations": all_violations,
                "summary": summary,
                "scope_source": resolved_scope.source,
                "scope_files": resolved_scope.files,
                "scanned_nodes": scanned_nodes,
                "elapsed_ms": elapsed_ms,
            })
        }
    };
    Ok(serde_json::to_string_pretty(&output)?)
}

/// Output verbosity mode for hermes_lint_architecture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Summary,
    Iterative,
    Full,
}

impl Mode {
    fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "iterative" => Mode::Iterative,
            "full" => Mode::Full,
            _ => Mode::Summary,
        }
    }
}

fn count_nodes(graph: &KnowledgeGraph) -> Result<u64> {
    let conn = graph.db().lock().map_err(|e| anyhow::anyhow!("{e}"))?;
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM nodes WHERE project_id = ?1",
        rusqlite::params![graph.project_id()],
        |row| row.get(0),
    )?;
    Ok(count as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn lint_on_empty_graph_returns_zero_violations() {
        let engine = crate::HermesEngine::in_memory("lint-test").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let result = tool_lint_architecture(&engine, dir.path(), &json!({})).unwrap();
        let v: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["summary"]["total"], 0);
    }

    #[test]
    fn lint_accepts_severity_filter() {
        let engine = crate::HermesEngine::in_memory("lint-sev").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let result = tool_lint_architecture(
            &engine,
            dir.path(),
            &json!({ "severity_min": "error", "mode": "full" }),
        )
        .unwrap();
        let v: Value = serde_json::from_str(&result).unwrap();
        assert!(v["violations"].is_array());
    }

    #[test]
    fn summary_mode_omits_full_violations_array() {
        let engine = crate::HermesEngine::in_memory("lint-summary").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let result = tool_lint_architecture(&engine, dir.path(), &json!({})).unwrap();
        let v: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["mode"], "summary");
        assert!(
            v["violations"].is_null(),
            "summary mode must not emit full violations array"
        );
        assert!(v["summary"]["by_rule_detail"].is_array());
        assert!(v["next"].is_string());
    }

    #[test]
    fn iterative_mode_requires_rule_id() {
        let engine = crate::HermesEngine::in_memory("lint-iter-noid").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let err = tool_lint_architecture(&engine, dir.path(), &json!({ "mode": "iterative" }))
            .unwrap_err();
        assert!(err.to_string().contains("rule_id"));
    }

    #[test]
    fn iterative_mode_returns_violations_for_one_rule() {
        let engine = crate::HermesEngine::in_memory("lint-iter").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let result = tool_lint_architecture(
            &engine,
            dir.path(),
            &json!({ "mode": "iterative", "rule_id": "SIZE-001", "max_violations": 10 }),
        )
        .unwrap();
        let v: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["mode"], "iterative");
        assert_eq!(v["rule_id"], "SIZE-001");
        assert!(v["violations"].is_array());
        assert!(v["total_for_rule"].is_number());
        assert!(v["truncated"].is_boolean());
    }

    #[test]
    fn full_mode_keeps_legacy_violations_array() {
        let engine = crate::HermesEngine::in_memory("lint-full").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let result =
            tool_lint_architecture(&engine, dir.path(), &json!({ "mode": "full" })).unwrap();
        let v: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["mode"], "full");
        assert!(v["violations"].is_array());
    }

    #[test]
    fn mode_parse_defaults_to_summary_for_unknown() {
        assert_eq!(Mode::parse("garbage"), Mode::Summary);
        assert_eq!(Mode::parse(""), Mode::Summary);
        assert_eq!(Mode::parse("FULL"), Mode::Full);
        assert_eq!(Mode::parse("Iterative"), Mode::Iterative);
    }

    #[test]
    fn lint_accepts_rule_filter() {
        let engine = crate::HermesEngine::in_memory("lint-rules").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let result =
            tool_lint_architecture(&engine, dir.path(), &json!({ "rules": ["SIZE-001"] })).unwrap();
        let v: Value = serde_json::from_str(&result).unwrap();
        assert!(v["summary"].is_object());
    }

    #[test]
    fn auto_scope_falls_back_to_all_on_non_git_dir() {
        let engine = crate::HermesEngine::in_memory("lint-auto-scope").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let result = tool_lint_architecture(&engine, dir.path(), &json!({})).unwrap();
        let v: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["scope_source"], "all");
        assert!(v["scope_files"].as_array().unwrap().is_empty());
    }

    #[test]
    fn explicit_scope_takes_precedence_over_auto_scope() {
        let engine = crate::HermesEngine::in_memory("lint-explicit-scope").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let result = tool_lint_architecture(
            &engine,
            dir.path(),
            &json!({ "scope": "src/foo.rs", "auto_scope": true }),
        )
        .unwrap();
        let v: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["scope_source"], "explicit");
        assert_eq!(v["scope_files"][0], "src/foo.rs");
    }

    #[test]
    fn auto_scope_disabled_forces_all_in_summary_mode() {
        let engine = crate::HermesEngine::in_memory("lint-noauto").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let result =
            tool_lint_architecture(&engine, dir.path(), &json!({ "auto_scope": false })).unwrap();
        let v: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["scope_source"], "all");
    }

    #[test]
    fn full_mode_does_not_auto_scope() {
        let engine = crate::HermesEngine::in_memory("lint-full-noauto").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let result =
            tool_lint_architecture(&engine, dir.path(), &json!({ "mode": "full" })).unwrap();
        let v: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["scope_source"], "all");
    }
}
