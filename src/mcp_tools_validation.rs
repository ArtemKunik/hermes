// src/mcp_tools_validation.rs
//
// TRACK-040 validation tools: environment variable presence checking and
// cross-codebase consistency reporting.
//
// Both tools read from the `config_registry` table which is populated during
// `hermes_index` by the env_scanner ingestion pass.

use anyhow::Result;
use serde_json::json;

use crate::HermesEngine;

/// Validate an environment variable name against the config_registry.
///
/// Returns `{valid: true}` if the name was discovered during indexing, or
/// `{valid: false, suggestions: [...]}` with the 5 closest known names
/// (by Levenshtein distance) so the caller can spot typos immediately.
pub fn tool_validate_env(engine: &HermesEngine, env_var: &str) -> Result<String> {
    let conn = engine.db().lock().map_err(|e| anyhow::anyhow!("{e}"))?;

    let exists: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM config_registry WHERE key = ?",
            [env_var],
            |row| row.get::<_, i64>(0),
        )
        .map(|c| c > 0)
        .unwrap_or(false);

    if exists {
        return Ok(serde_json::to_string_pretty(&json!({
            "valid": true,
            "suggestions": []
        }))?);
    }

    // Collect all known keys for Levenshtein-based suggestions.
    let mut stmt = conn.prepare("SELECT DISTINCT key FROM config_registry ORDER BY key")?;
    let known: Vec<String> = stmt
        .query_map([], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;

    let mut ranked: Vec<(String, usize)> = known
        .into_iter()
        .map(|k| {
            let dist = strsim::levenshtein(env_var, &k);
            (k, dist)
        })
        .collect();
    ranked.sort_by_key(|(_, d)| *d);
    ranked.truncate(5);
    let suggestions: Vec<String> = ranked.into_iter().map(|(k, _)| k).collect();

    Ok(serde_json::to_string_pretty(&json!({
        "valid": false,
        "suggestions": suggestions
    }))?)
}

/// Check consistency of environment variables across the whole codebase.
///
/// After `hermes_index` runs, the config_registry holds every env var name
/// that was either *defined* (seen in .env / YAML / Markdown tables) or
/// *used* (accessed via `std::env::var`, `process.env.X`, `os.getenv`, etc.).
///
/// Returns three categories:
/// - `unknown_variables`  — used in code but never defined (potential typo or missing .env entry)
/// - `unused_variables`   — defined but never accessed in code (dead config)
/// - `consistent_variables` — both defined and used
pub fn tool_check_consistency(engine: &HermesEngine) -> Result<String> {
    let conn = engine.db().lock().map_err(|e| anyhow::anyhow!("{e}"))?;

    let mut stmt = conn.prepare(
        "SELECT key, is_defined, is_used FROM config_registry ORDER BY key",
    )?;

    let rows: Vec<(String, bool, bool)> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)? != 0,
                row.get::<_, i64>(2)? != 0,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    // Pre-collect defined keys for "did you mean" suggestions on unknown vars.
    let defined_keys: Vec<&str> = rows
        .iter()
        .filter(|(_, is_def, _)| *is_def)
        .map(|(k, _, _)| k.as_str())
        .collect();

    let mut unknown = Vec::new();
    let mut unused = Vec::new();
    let mut consistent = Vec::new();

    for (key, is_defined, is_used) in &rows {
        match (is_defined, is_used) {
            (false, true) => {
                let suggestion: Option<String> = defined_keys
                    .iter()
                    .filter_map(|&k| {
                        let d = strsim::levenshtein(key.as_str(), k);
                        if d <= 3 {
                            Some((k.to_string(), d))
                        } else {
                            None
                        }
                    })
                    .min_by_key(|(_, d)| *d)
                    .map(|(k, _)| k);
                unknown.push(json!({ "variable": key, "suggestion": suggestion }));
            }
            (true, false) => {
                unused.push(json!({ "variable": key }));
            }
            _ => {
                consistent.push(json!({ "variable": key }));
            }
        }
    }

    Ok(serde_json::to_string_pretty(&json!({
        "status": if unknown.is_empty() && unused.is_empty() { "clear" } else { "issues_found" },
        "summary": {
            "unknown_count":    unknown.len(),
            "unused_count":     unused.len(),
            "consistent_count": consistent.len()
        },
        "unknown_variables":    unknown,
        "unused_variables":     unused,
        "consistent_variables": consistent
    }))?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HermesEngine;

    fn engine_with_registry(entries: &[(&str, bool, bool)]) -> HermesEngine {
        let engine = HermesEngine::in_memory("test").unwrap();
        let conn = engine.db().lock().unwrap();
        for (key, is_def, is_used) in entries {
            conn.execute(
                "INSERT INTO config_registry (key, is_defined, is_used) VALUES (?1, ?2, ?3)",
                rusqlite::params![key, *is_def as i64, *is_used as i64],
            )
            .unwrap();
        }
        drop(conn);
        engine
    }

    #[test]
    fn test_validate_env_known_key_returns_valid() {
        let engine = engine_with_registry(&[("MY_KEY", false, true)]);
        let result: serde_json::Value =
            serde_json::from_str(&tool_validate_env(&engine, "MY_KEY").unwrap()).unwrap();
        assert_eq!(result["valid"], true);
    }

    #[test]
    fn test_validate_env_unknown_key_suggests_nearest() {
        let engine = engine_with_registry(&[("DATABASE_URL", true, true)]);
        let result: serde_json::Value =
            serde_json::from_str(&tool_validate_env(&engine, "DATABASE_URI").unwrap()).unwrap();
        assert_eq!(result["valid"], false);
        let suggestions = result["suggestions"].as_array().unwrap();
        assert!(suggestions.iter().any(|s| s.as_str() == Some("DATABASE_URL")));
    }

    #[test]
    fn test_check_consistency_detects_unknown() {
        let engine = engine_with_registry(&[
            ("DEFINED_KEY", true, true),
            ("ORPHAN_KEY", false, true), // used but not defined
        ]);
        let result: serde_json::Value =
            serde_json::from_str(&tool_check_consistency(&engine).unwrap()).unwrap();
        assert_eq!(result["status"], "issues_found");
        assert_eq!(result["summary"]["unknown_count"], 1);
        let unknown = result["unknown_variables"].as_array().unwrap();
        assert_eq!(unknown[0]["variable"], "ORPHAN_KEY");
    }

    #[test]
    fn test_check_consistency_detects_unused() {
        let engine = engine_with_registry(&[
            ("USED_KEY", true, true),
            ("DEAD_KEY", true, false), // defined but never used
        ]);
        let result: serde_json::Value =
            serde_json::from_str(&tool_check_consistency(&engine).unwrap()).unwrap();
        assert_eq!(result["summary"]["unused_count"], 1);
        let unused = result["unused_variables"].as_array().unwrap();
        assert_eq!(unused[0]["variable"], "DEAD_KEY");
    }

    #[test]
    fn test_check_consistency_clear_when_all_consistent() {
        let engine = engine_with_registry(&[("OK_VAR", true, true)]);
        let result: serde_json::Value =
            serde_json::from_str(&tool_check_consistency(&engine).unwrap()).unwrap();
        assert_eq!(result["status"], "clear");
    }
}
