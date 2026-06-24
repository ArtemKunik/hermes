// tools/hermes-engine/src/arch_rules/query_rules.rs
// TRACK-045: QUERY-001 — String interpolation in Cosmos/DB queries.

use anyhow::Result;
use regex::Regex;
use rusqlite::params;
use std::path::Path;

use crate::arch_rules::{ArchRule, Severity, Violation};
use crate::graph::KnowledgeGraph;

fn resolve_path(project_root: &Path, file_path: &str) -> Option<std::path::PathBuf> {
    let p = std::path::Path::new(file_path);
    if p.is_absolute() && p.exists() { return Some(p.to_path_buf()); }
    let joined = project_root.join(file_path);
    if joined.exists() { Some(joined) } else { None }
}

fn is_store_file(file_path: &str) -> bool {
    let norm = file_path.replace('\\', "/").to_lowercase();
    norm.contains("store_")
        || norm.contains("/store/")
        || norm.contains("cosmos_")
        || norm.contains("_repository")
        || norm.contains("_repo.")
}

// ---------------------------------------------------------------------------
// QUERY-001: format! macro used in store module containing SQL/Cosmos keywords
// ---------------------------------------------------------------------------

pub struct StringInterpolationQueryRule;

impl ArchRule for StringInterpolationQueryRule {
    fn id(&self) -> &str { "QUERY-001" }
    fn severity(&self) -> Severity { Severity::Error }
    fn description(&self) -> &str {
        "String interpolation (format!) in store/query module — use parameterized queries instead"
    }

    fn evaluate(&self, graph: &KnowledgeGraph, project_root: &Path) -> Result<Vec<Violation>> {
        // Match format! macro calls that build SQL-like strings
        let format_pattern = Regex::new(r#"format!\s*\("#).unwrap();
        // File-level: broad set to detect files that touch a DB at all
        let file_sql_keywords = Regex::new(
            r#"(?i)\b(SELECT|WHERE|INSERT|UPDATE|DELETE|FROM|JOIN|ORDER BY|GROUP BY)\b"#,
        ).unwrap();
        // Per-line: stricter — SELECT and WHERE almost never appear in non-SQL Rust code,
        // unlike INSERT/DELETE/JOIN/FROM which clash with .insert()/.delete()/.join()/use..from.
        let line_sql_keywords = Regex::new(
            r#"(?i)\b(SELECT|WHERE|INSERT\s+INTO|UPDATE\s+\w+\s+SET|DELETE\s+FROM)\b"#,
        ).unwrap();

        let conn = graph.db().lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut stmt = conn.prepare(
            "SELECT DISTINCT file_path FROM nodes
             WHERE project_id = ?1
               AND node_type = 'file'
               AND file_path LIKE '%.rs'
               AND file_path NOT LIKE '%node_modules%'",
        )?;
        let paths: Vec<String> = stmt
            .query_map(params![graph.project_id()], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .filter(|fp: &String| is_store_file(fp))
            .collect();
        drop(stmt);
        drop(conn);

        let mut violations = Vec::new();
        for fp in paths {
            let Some(abs_path) = resolve_path(project_root, &fp) else { continue };
            let Ok(content) = std::fs::read_to_string(&abs_path) else { continue };

            // Check if this file contains SQL/Cosmos query keywords
            if !file_sql_keywords.is_match(&content) { continue; }

            for (i, line) in content.lines().enumerate() {
                let trimmed = line.trim();
                if trimmed.starts_with("//") { continue; }
                if format_pattern.is_match(line) && line_sql_keywords.is_match(line) {
                    violations.push(
                        Violation::new(self.id(), self.severity(), &fp,
                            format!("String interpolation in query on line {} — SQL injection risk", i + 1))
                            .with_lines((i + 1) as u32, (i + 1) as u32)
                            .with_suggestion("Use parameterized queries: rusqlite params![] or Cosmos SDK query parameters"),
                    );
                }
            }
        }
        Ok(violations)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_rule_id() {
        assert_eq!(StringInterpolationQueryRule.id(), "QUERY-001");
        assert_eq!(StringInterpolationQueryRule.severity(), Severity::Error);
    }
}
