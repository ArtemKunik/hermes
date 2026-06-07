// tools/hermes-engine/src/arch_rules/size_rules.rs
// TRACK-045: SIZE-001 (file >300 lines) and SIZE-002 (method >50 lines).

use anyhow::Result;
use rusqlite::params;
use std::path::Path;

use crate::arch_rules::{ArchRule, Severity, Violation};
use crate::graph::KnowledgeGraph;

// ---------------------------------------------------------------------------
// SIZE-001: File exceeds 300 lines
// ---------------------------------------------------------------------------

pub struct FileSizeRule;

impl ArchRule for FileSizeRule {
    fn id(&self) -> &str {
        "SIZE-001"
    }
    fn severity(&self) -> Severity {
        Severity::Error
    }
    fn description(&self) -> &str {
        "Source file exceeds 300 lines (AGENTS.md hard limit)"
    }

    fn evaluate(&self, graph: &KnowledgeGraph, _root: &Path) -> Result<Vec<Violation>> {
        let conn = graph.db().lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        // File nodes track the overall line span of the file.
        let mut stmt = conn.prepare(
            "SELECT name, file_path, start_line, end_line
             FROM nodes
             WHERE project_id = ?1
               AND node_type = 'file'
               AND end_line IS NOT NULL AND start_line IS NOT NULL
               AND (end_line - start_line) > 300
               AND file_path NOT LIKE '%node_modules%'
               AND file_path NOT LIKE '%.min.%'
               AND file_path NOT LIKE '%generated%'",
        )?;
        let rows: Vec<(String, String, i64, i64)> = stmt
            .query_map(params![graph.project_id()], |row| {
                Ok((
                    row.get::<_, String>(0).unwrap_or_default(),
                    row.get::<_, String>(1).unwrap_or_default(),
                    row.get::<_, i64>(2).unwrap_or(0),
                    row.get::<_, i64>(3).unwrap_or(0),
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);
        drop(conn);

        Ok(rows
            .into_iter()
            .map(|(name, fp, start, end)| {
                let lines = end - start;
                Violation::new(
                    self.id(),
                    self.severity(),
                    &fp,
                    format!(
                        "`{name}` has {lines} lines — AGENTS.md limits source files to 300 lines"
                    ),
                )
                .with_lines(1, end as u32)
                .with_suggestion("Refactor: extract cohesive blocks into separate modules")
            })
            .collect())
    }
}

// ---------------------------------------------------------------------------
// SIZE-002: Method/function exceeds 50 lines
// ---------------------------------------------------------------------------

pub struct MethodSizeRule;

impl ArchRule for MethodSizeRule {
    fn id(&self) -> &str {
        "SIZE-002"
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }
    fn description(&self) -> &str {
        "Function or method exceeds 50 lines (AGENTS.md hard limit)"
    }

    fn evaluate(&self, graph: &KnowledgeGraph, _root: &Path) -> Result<Vec<Violation>> {
        let conn = graph.db().lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut stmt = conn.prepare(
            "SELECT name, file_path, start_line, end_line
             FROM nodes
             WHERE project_id = ?1
               AND node_type IN ('function', 'impl')
               AND end_line IS NOT NULL AND start_line IS NOT NULL
               AND (end_line - start_line) > 50
               AND file_path NOT LIKE '%node_modules%'
               AND file_path NOT LIKE '%generated%'",
        )?;
        let rows: Vec<(String, String, i64, i64)> = stmt
            .query_map(params![graph.project_id()], |row| {
                Ok((
                    row.get::<_, String>(0).unwrap_or_default(),
                    row.get::<_, String>(1).unwrap_or_default(),
                    row.get::<_, i64>(2).unwrap_or(0),
                    row.get::<_, i64>(3).unwrap_or(0),
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);
        drop(conn);

        Ok(rows
            .into_iter()
            .map(|(sym, fp, start, end)| {
                let lines = end - start;
                Violation::new(
                    self.id(),
                    self.severity(),
                    &fp,
                    format!("`{sym}` has {lines} lines — AGENTS.md limits methods to 50 lines"),
                )
                .with_lines(start as u32, end as u32)
                .with_symbol(&sym)
                .with_suggestion("Extract helper functions to reduce method body size")
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_rule_ids() {
        assert_eq!(FileSizeRule.id(), "SIZE-001");
        assert_eq!(MethodSizeRule.id(), "SIZE-002");
    }
}
