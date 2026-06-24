// tools/hermes-engine/src/bin/hermes/handlers_advanced.rs
use anyhow::Result;
use hermes_engine::{mcp_commit, mcp_lint, mcp_heal};
use hermes_engine::HermesEngine;

pub fn cmd_lint_architecture(
    engine: &HermesEngine,
    project_root: &std::path::Path,
    scope: Option<&str>,
    severity_min: Option<&str>,
    rules: Option<&str>,
) -> Result<()> {
    let mut lint_args = serde_json::json!({ "mode": "full" });
    if let Some(s) = scope {
        lint_args["scope"] = serde_json::Value::String(s.to_string());
    }
    if let Some(sev) = severity_min {
        lint_args["severity_min"] = serde_json::Value::String(sev.to_string());
    }
    if let Some(r) = rules {
        let ids: Vec<serde_json::Value> = r.split(',')
            .map(|s| serde_json::Value::String(s.trim().to_string()))
            .collect();
        lint_args["rules"] = serde_json::Value::Array(ids);
    }

    let out = mcp_lint::tool_lint_architecture(engine, project_root, &lint_args)?;
    println!("{out}");
    // Exit non-zero if any error-severity violations found
    let v: serde_json::Value = serde_json::from_str(&out)?;
    let errors = v["summary"]["by_severity"]["error"].as_u64().unwrap_or(0);
    if errors > 0 {
        std::process::exit(1);
    }
    Ok(())
}

pub fn cmd_heal_violations(
    engine: &HermesEngine,
    project_root: &std::path::Path,
    args: &[String],
) -> Result<()> {
    let mut heal_args = serde_json::json!({});
    let mut i = 2usize;
    while i < args.len() {
        match args[i].as_str() {
            "--scope" => {
                if let Some(v) = args.get(i + 1) {
                    heal_args["scope"] = serde_json::Value::String(v.clone());
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "--severity-min" => {
                if let Some(v) = args.get(i + 1) {
                    heal_args["severity_min"] = serde_json::Value::String(v.clone());
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "--rules" => {
                if let Some(v) = args.get(i + 1) {
                    let ids: Vec<serde_json::Value> = v
                        .split(',')
                        .map(|s| serde_json::Value::String(s.trim().to_string()))
                        .collect();
                    heal_args["rules"] = serde_json::Value::Array(ids);
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "--max-items" => {
                if let Some(v) = args.get(i + 1) {
                    if let Ok(parsed) = v.parse::<u64>() {
                        heal_args["max_items"] = serde_json::Value::Number(parsed.into());
                    }
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "--apply" => {
                heal_args["dry_run"] = serde_json::Value::Bool(false);
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }
    let out = mcp_heal::tool_heal_violations(engine, project_root, &heal_args)?;
    println!("{out}");
    Ok(())
}

pub fn cmd_prepare_commit_message(args: &[String]) -> Result<()> {
    let (subject, input) = mcp_commit::input_from_cli_args(args)?;
    let message = mcp_commit::render_commit_message(&subject, &input);
    println!("{message}");
    Ok(())
}
