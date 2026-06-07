// tools/hermes-engine/src/mcp_tools_validation.rs
//
// TRACK-040 validation tools: env var and symbol name validation.
// Extracted from mcp_tools.rs for 300-line file limit compliance.
// HP18: adds stale_index_hint when missed symbols may be due to a stale index.

use anyhow::Result;
use serde_json::json;

use crate::accounting::Accountant;
use crate::HermesEngine;

/// TRACK-040 Phase 1: Validate environment variable against config_registry
pub fn tool_validate_env(engine: &HermesEngine, env_var: &str) -> Result<String> {
    let db = engine.read_db().lock().unwrap_or_else(|e| e.into_inner());
    tool_validate_env_with_conn(engine, &db, env_var)
}

pub fn tool_validate_env_with_conn(
    engine: &HermesEngine,
    conn: &rusqlite::Connection,
    env_var: &str,
) -> Result<String> {
    let mut stmt = conn.prepare("SELECT key FROM config_registry WHERE key = ?")?;
    let exists = stmt.exists([env_var])?;
    drop(stmt);

    if exists {
        let acct = Accountant::from_conn(conn, engine.project_id(), engine.session_id());
        let _ = acct.record_query(&format!("validate_env:{env_var}"), 30, 0, 30);
        return Ok(serde_json::to_string_pretty(&json!({
            "valid": true,
            "suggestions": []
        }))?);
    }

    // Get all known env vars for suggestions
    let mut stmt = conn.prepare("SELECT key FROM config_registry ORDER BY key")?;
    let known_vars: Vec<String> = stmt
        .query_map([], |row: &rusqlite::Row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let scanned = known_vars.len() as u64;
    drop(stmt);

    // Find closest matches using Levenshtein distance
    let mut suggestions: Vec<(String, usize)> = known_vars
        .into_iter()
        .map(|known: String| (known.clone(), strsim::levenshtein(env_var, &known)))
        .collect();

    suggestions.sort_by_key(|(_, dist)| *dist);
    suggestions.truncate(5);

    let suggestions: Vec<String> = suggestions.into_iter().map(|(var, _)| var).collect();

    let acct = Accountant::from_conn(conn, engine.project_id(), engine.session_id());
    let ptr_tokens = 30u64;
    let _ = acct.record_query(
        &format!("validate_env:{env_var}"),
        ptr_tokens,
        0,
        scanned.saturating_mul(3).max(ptr_tokens),
    );

    Ok(serde_json::to_string_pretty(&json!({
        "valid": false,
        "suggestions": suggestions
    }))?)
}

/// TRACK-040 Phase 4: Validate symbol names against the knowledge graph.
///
/// For each symbol in the input slice, checks whether a node with that name
/// exists in the graph (case-insensitive). If not found, returns the top-5
/// nearest known symbol names by Levenshtein distance.
pub fn tool_validate_symbols(engine: &HermesEngine, symbols: &[&str]) -> Result<String> {
    let db = engine.read_db().lock().unwrap_or_else(|e| e.into_inner());
    tool_validate_symbols_with_conn(engine, &db, symbols)
}

pub fn tool_validate_symbols_with_conn(
    engine: &HermesEngine,
    conn: &rusqlite::Connection,
    symbols: &[&str],
) -> Result<String> {
    let project_id = engine.project_id();

    // Load all known symbol names once — cheaper than N round-trips.
    let mut stmt = conn.prepare(
        "SELECT DISTINCT name FROM nodes \
         WHERE project_id = ? AND name IS NOT NULL AND name != '' \
         ORDER BY name",
    )?;
    let known_names: Vec<String> = stmt
        .query_map([project_id], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let known_len = known_names.len();
    drop(stmt);

    let mut results = Vec::new();
    for &symbol in symbols {
        let symbol_lower = symbol.to_lowercase();
        let valid = known_names.iter().any(|n| n.to_lowercase() == symbol_lower);

        let suggestions: Vec<String> = if valid {
            vec![]
        } else {
            let threshold = (symbol.len() / 2 + 2).min(8);
            let mut candidates: Vec<(String, usize)> = known_names
                .iter()
                .map(|n| {
                    let dist = strsim::levenshtein(&symbol_lower, &n.to_lowercase());
                    (n.clone(), dist)
                })
                .collect();
            candidates.sort_by_key(|(_, d)| *d);
            candidates.truncate(5);
            candidates
                .into_iter()
                .filter(|(_, d)| *d <= threshold)
                .map(|(n, _)| n)
                .collect()
        };

        results.push(json!({
            "symbol": symbol,
            "valid": valid,
            "suggestions": suggestions,
        }));
    }

    let acct = Accountant::from_conn(conn, engine.project_id(), engine.session_id());
    let ptr_tokens = (symbols.len() as u64).saturating_mul(30) + 20;
    let traditional = (known_len as u64).saturating_mul(5).max(ptr_tokens);
    let _ = acct.record_query(
        &format!("validate_symbols:{}", symbols.first().unwrap_or(&"")),
        ptr_tokens,
        0,
        traditional,
    );

    let any_invalid = results.iter().any(|r| r["valid"] == false);
    let stale_hint = any_invalid && has_stale_indexed_files(conn, project_id);

    Ok(serde_json::to_string_pretty(&json!({
        "results": results,
        "stale_index_hint": stale_hint,
    }))?)
}

/// Returns true when at least one file in `file_hashes` was modified on disk
/// after its `indexed_at` timestamp — a reliable indicator of a stale index.
///
/// Checks only the 50 most recently indexed files to bound I/O cost.
fn has_stale_indexed_files(conn: &rusqlite::Connection, project_id: &str) -> bool {
    let Ok(mut stmt) = conn.prepare(
        "SELECT file_path, indexed_at FROM file_hashes \
         WHERE project_id = ? ORDER BY indexed_at DESC LIMIT 50",
    ) else {
        return false;
    };
    let Ok(rows) = stmt.query_map([project_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }) else {
        return false;
    };
    for row in rows.flatten() {
        let (file_path, indexed_at_str) = row;
        let Ok(indexed_at) =
            chrono::NaiveDateTime::parse_from_str(&indexed_at_str, "%Y-%m-%d %H:%M:%S")
        else {
            continue;
        };
        let indexed_at_utc: chrono::DateTime<chrono::Utc> =
            chrono::DateTime::from_naive_utc_and_offset(indexed_at, chrono::Utc);
        if let Ok(meta) = std::fs::metadata(&file_path) {
            if let Ok(mtime) = meta.modified() {
                let mtime_utc: chrono::DateTime<chrono::Utc> = mtime.into();
                if mtime_utc > indexed_at_utc {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{graph::KnowledgeGraph, graph_builders::NodeBuilder, HermesEngine};

    fn engine_with_symbols(symbols: &[&str]) -> HermesEngine {
        let engine = HermesEngine::in_memory("test-project").unwrap();
        let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
        for &sym in symbols {
            let node = NodeBuilder::new("test-project")
                .name(sym)
                .file_path("test.rs")
                .build();
            graph.add_node(&node).unwrap();
        }
        engine
    }

    #[test]
    fn test_validate_symbols_valid_symbol_returns_true() {
        let engine = engine_with_symbols(&["ingest_file", "XrefExtractor"]);
        let result: serde_json::Value =
            serde_json::from_str(&tool_validate_symbols(&engine, &["ingest_file"]).unwrap())
                .unwrap();
        assert_eq!(result["results"][0]["valid"], true);
        assert!(result["results"][0]["suggestions"]
            .as_array()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn test_validate_symbols_case_insensitive_match() {
        let engine = engine_with_symbols(&["IngestFile"]);
        let result: serde_json::Value =
            serde_json::from_str(&tool_validate_symbols(&engine, &["ingestfile"]).unwrap())
                .unwrap();
        assert_eq!(result["results"][0]["valid"], true);
    }

    #[test]
    fn test_validate_symbols_unknown_suggests_nearest() {
        let engine = engine_with_symbols(&["ingest_file", "xref_extractor", "ast_chunker"]);
        let result: serde_json::Value =
            serde_json::from_str(&tool_validate_symbols(&engine, &["ingest_fil"]).unwrap())
                .unwrap();
        let entry = &result["results"][0];
        assert_eq!(entry["valid"], false);
        let suggestions = entry["suggestions"].as_array().unwrap();
        assert!(
            suggestions
                .iter()
                .any(|s| s.as_str() == Some("ingest_file")),
            "Expected 'ingest_file' in suggestions, got: {suggestions:?}",
        );
    }

    #[test]
    fn test_validate_symbols_no_stale_hint_when_all_valid() {
        let engine = engine_with_symbols(&["existing_sym"]);
        let result: serde_json::Value =
            serde_json::from_str(&tool_validate_symbols(&engine, &["existing_sym"]).unwrap())
                .unwrap();
        assert_eq!(result["stale_index_hint"], false);
    }

    #[test]
    fn test_validate_symbols_no_stale_hint_when_no_file_hashes() {
        // Miss with no file_hashes entries → hint stays false
        let engine = engine_with_symbols(&["some_sym"]);
        let result: serde_json::Value =
            serde_json::from_str(&tool_validate_symbols(&engine, &["missing_sym"]).unwrap())
                .unwrap();
        assert_eq!(result["results"][0]["valid"], false);
        assert_eq!(result["stale_index_hint"], false);
    }

    #[test]
    fn test_validate_symbols_stale_hint_true_when_file_modified_after_index() {
        let engine = engine_with_symbols(&["some_sym"]);
        let tmpfile = std::env::temp_dir().join("hermes_stale_test_hp18.rs");
        std::fs::write(&tmpfile, b"fn stub() {}").unwrap();
        {
            let db = engine.read_db().lock().unwrap();
            db.execute(
                "INSERT OR REPLACE INTO file_hashes \
                 (file_path, project_id, indexed_at) \
                 VALUES (?, ?, datetime('now', '-2 hours'))",
                rusqlite::params![tmpfile.to_str().unwrap(), "test-project"],
            )
            .unwrap();
        }
        let result: serde_json::Value =
            serde_json::from_str(&tool_validate_symbols(&engine, &["missing_sym"]).unwrap())
                .unwrap();
        let _ = std::fs::remove_file(&tmpfile);
        assert_eq!(result["results"][0]["valid"], false);
        assert_eq!(result["stale_index_hint"], true);
    }

    #[test]
    fn test_validate_symbols_empty_graph_unknown_no_suggestions() {
        let engine = engine_with_symbols(&[]);
        let result: serde_json::Value =
            serde_json::from_str(&tool_validate_symbols(&engine, &["anything"]).unwrap()).unwrap();
        let entry = &result["results"][0];
        assert_eq!(entry["valid"], false);
        assert!(entry["suggestions"].as_array().unwrap().is_empty());
    }
}
