// tools/hermes-engine/src/mcp_quality_drift.rs
// TRACK-NEW: Lint baseline snapshot + drift detection.
//
// tool_quality_baseline — snapshot current arch-lint violation set into quality-state.json.
// tool_quality_drift    — compare current violations against the stored baseline and return delta.

use anyhow::Result;
use chrono::Utc;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::{
    arch_rules::{default_rules, Severity},
    graph::KnowledgeGraph,
    quality::state::{load, save, LintBaseline},
    HermesEngine,
};

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Run all arch rules, return collected violations.
fn collect_violations(
    engine: &HermesEngine,
    project_root: &Path,
) -> Vec<crate::arch_rules::Violation> {
    let graph = KnowledgeGraph::new(engine.read_db().clone(), engine.project_id());
    let rules = default_rules();
    let mut out = Vec::new();
    for rule in &rules {
        if rule.severity() < Severity::Error {
            continue;
        }
        match rule.evaluate(&graph, project_root) {
            Ok(vs) => out.extend(vs),
            Err(e) => eprintln!("[quality-drift] rule {} failed: {e}", rule.id()),
        }
    }
    out
}

/// Normalise a file path to a project-relative forward-slash string.
fn rel_path(file_path: &str, project_root: &Path) -> String {
    let norm = file_path.replace('\\', "/");
    let root = project_root.to_string_lossy().replace('\\', "/");
    norm.strip_prefix(&format!("{root}/"))
        .or_else(|| norm.strip_prefix(&root))
        .unwrap_or(&norm)
        .to_string()
}

/// Build a deterministic fingerprint for a violation.
fn fingerprint(v: &crate::arch_rules::Violation, project_root: &Path) -> String {
    let file = rel_path(&v.file_path, project_root);
    let line = v.line_start.unwrap_or(0);
    format!("{}:{}:{}", v.rule_id, file, line)
}

/// Read current HEAD commit sha (best-effort, returns None on failure).
fn current_commit() -> Option<String> {
    std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

// ---------------------------------------------------------------------------
// Public tool functions
// ---------------------------------------------------------------------------

/// Snapshot current arch-lint violations as the drift baseline.
pub fn tool_quality_baseline(
    engine: &HermesEngine,
    project_root: &Path,
    _args: &Value,
) -> Result<String> {
    let violations = collect_violations(engine, project_root);

    let mut by_rule: HashMap<String, usize> = HashMap::new();
    let mut keys: Vec<String> = Vec::with_capacity(violations.len());
    for v in &violations {
        *by_rule.entry(v.rule_id.clone()).or_default() += 1;
        keys.push(fingerprint(v, project_root));
    }
    keys.sort();
    keys.dedup();

    let baseline = LintBaseline {
        recorded_at: Utc::now().to_rfc3339(),
        commit: current_commit(),
        total: violations.len(),
        by_rule: by_rule.clone(),
        violation_keys: keys,
    };

    let mut state = load(project_root)?;
    state.lint_baseline = Some(baseline);
    save(project_root, &state)?;

    Ok(serde_json::to_string_pretty(&json!({
        "status": "baseline_recorded",
        "total": violations.len(),
        "by_rule": by_rule,
        "commit": state.lint_baseline.as_ref().and_then(|b| b.commit.as_deref()),
        "recorded_at": state.lint_baseline.as_ref().map(|b| b.recorded_at.as_str()),
    }))?)
}

/// Compare current violations against the stored baseline.
pub fn tool_quality_drift(
    engine: &HermesEngine,
    project_root: &Path,
    _args: &Value,
) -> Result<String> {
    let state = load(project_root)?;
    let Some(baseline) = &state.lint_baseline else {
        return Ok(json!({
            "status": "no_baseline",
            "message": "Run `hermes quality-baseline` first to record a baseline."
        })
        .to_string());
    };

    let violations = collect_violations(engine, project_root);
    let baseline_keys: HashSet<&str> = baseline.violation_keys.iter().map(String::as_str).collect();

    let mut new_violations = Vec::new();
    let mut current_keys: HashSet<String> = HashSet::new();
    let mut by_rule_now: HashMap<String, usize> = HashMap::new();

    for v in &violations {
        let key = fingerprint(v, project_root);
        by_rule_now.entry(v.rule_id.clone()).or_default();
        *by_rule_now.get_mut(&v.rule_id).unwrap() += 1;
        if !baseline_keys.contains(key.as_str()) {
            new_violations.push(json!({
                "rule_id": v.rule_id,
                "file":    rel_path(&v.file_path, project_root),
                "line":    v.line_start,
                "message": v.message,
            }));
        }
        current_keys.insert(key);
    }

    let fixed_count = baseline
        .violation_keys
        .iter()
        .filter(|k| !current_keys.contains(k.as_str()))
        .count();

    let new_count = new_violations.len();
    let trend = match new_count.cmp(&fixed_count) {
        std::cmp::Ordering::Greater => "degrading",
        std::cmp::Ordering::Less => "improving",
        std::cmp::Ordering::Equal => "stable",
    };

    Ok(serde_json::to_string_pretty(&json!({
        "trend":         trend,
        "new_count":     new_count,
        "fixed_count":   fixed_count,
        "total_now":     violations.len(),
        "total_baseline": baseline.total,
        "by_rule_now":   by_rule_now,
        "by_rule_baseline": baseline.by_rule,
        "baseline_commit":  baseline.commit,
        "baseline_recorded_at": baseline.recorded_at,
        "new_violations": new_violations,
    }))?)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn make_engine(label: &str) -> HermesEngine {
        HermesEngine::in_memory(label).expect("engine")
    }

    #[test]
    fn baseline_on_empty_project_records_zero() {
        let dir = tempdir().unwrap();
        let engine = make_engine("drift-baseline-empty");
        let out = tool_quality_baseline(&engine, dir.path(), &json!({})).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["status"], "baseline_recorded");
        assert_eq!(v["total"], 0);
    }

    #[test]
    fn drift_without_baseline_returns_no_baseline() {
        let dir = tempdir().unwrap();
        let engine = make_engine("drift-no-baseline");
        let out = tool_quality_drift(&engine, dir.path(), &json!({})).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["status"], "no_baseline");
    }

    #[test]
    fn drift_after_baseline_reports_stable() {
        let dir = tempdir().unwrap();
        let engine = make_engine("drift-stable");
        tool_quality_baseline(&engine, dir.path(), &json!({})).unwrap();
        let out = tool_quality_drift(&engine, dir.path(), &json!({})).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["trend"], "stable");
        assert_eq!(v["new_count"], 0);
        assert_eq!(v["fixed_count"], 0);
    }
}
