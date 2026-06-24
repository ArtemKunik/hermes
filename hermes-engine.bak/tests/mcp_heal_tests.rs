use hermes_engine::mcp_heal::tool_heal_violations;
use hermes_engine::HermesEngine;
use serde_json::json;

#[test]
fn heal_tool_enforces_gpt5_mini_model_lock() {
    let engine = HermesEngine::in_memory("heal-model-lock").unwrap();
    let dir = tempfile::tempdir().unwrap();

    let err = tool_heal_violations(
        &engine,
        dir.path(),
        &json!({
            "model": "gpt-5"
        }),
    )
    .unwrap_err();

    assert!(
        err.to_string().contains("gpt-5-mini"),
        "tool must hard-lock healing model to gpt-5-mini"
    );
}

#[test]
fn heal_tool_selects_only_initial_eligible_rules() {
    let engine = HermesEngine::in_memory("heal-eligible-rules").unwrap();
    let dir = tempfile::tempdir().unwrap();

    let out = tool_heal_violations(
        &engine,
        dir.path(),
        &json!({
            "dry_run": true,
            "max_items": 10,
            "lint_payload": {
                "violations": [
                    {
                        "rule_id": "SAFETY-001",
                        "severity": "warning",
                        "file_path": "tools/hermes-engine/src/foo.rs",
                        "line_start": 10,
                        "line_end": 10,
                        "message": "unwrap in production",
                        "suggestion": "replace with ?",
                        "symbol": "foo"
                    },
                    {
                        "rule_id": "SIZE-001",
                        "severity": "error",
                        "file_path": "tools/hermes-engine/src/bar.rs",
                        "line_start": 1,
                        "line_end": 400,
                        "message": "file too large",
                        "suggestion": "split module",
                        "symbol": null
                    },
                    {
                        "rule_id": "SAFETY-003",
                        "severity": "warning",
                        "file_path": "tools/ccterm/src/web_ui/sample.ts",
                        "line_start": 24,
                        "line_end": 24,
                        "message": "any without SAFETY",
                        "suggestion": "add type",
                        "symbol": null
                    }
                ],
                "summary": {
                    "total": 3,
                    "by_severity": {"error": 1, "warning": 2, "info": 0},
                    "by_rule": {"SAFETY-001": 1, "SIZE-001": 1, "SAFETY-003": 1}
                }
            }
        }),
    )
    .unwrap();

    let payload: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(payload["model"], "gpt-5-mini");
    assert_eq!(payload["dry_run"], true);
    assert_eq!(payload["eligible_count"], 2);
    let candidates = payload["candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 2);
    let mut rules = candidates
        .iter()
        .filter_map(|c| c["rule_id"].as_str())
        .map(|s| s.to_string())
        .collect::<Vec<_>>();
    rules.sort();
    assert_eq!(rules, vec!["SAFETY-001".to_string(), "SAFETY-003".to_string()]);
}
