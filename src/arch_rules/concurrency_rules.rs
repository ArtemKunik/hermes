// tools/hermes-engine/src/arch_rules/concurrency_rules.rs
// TRACK-045: CONCURRENCY-001 — Rc usage in async context (should use Arc).

use anyhow::Result;
use regex::Regex;
use rusqlite::params;
use std::path::Path;

use crate::arch_rules::{ArchRule, Severity, Violation};
use crate::graph::KnowledgeGraph;

fn resolve_path(project_root: &Path, file_path: &str) -> Option<std::path::PathBuf> {
    let p = std::path::Path::new(file_path);
    if p.is_absolute() && p.exists() {
        return Some(p.to_path_buf());
    }
    let joined = project_root.join(file_path);
    if joined.exists() {
        Some(joined)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// CONCURRENCY-001: Rc usage (use Arc in async Rust files)
// ---------------------------------------------------------------------------

pub struct RcUsageRule;

impl ArchRule for RcUsageRule {
    fn id(&self) -> &str {
        "CONCURRENCY-001"
    }
    fn severity(&self) -> Severity {
        Severity::Error
    }
    fn description(&self) -> &str {
        "Use Arc<T> for thread-safe reference counting instead of non-thread-safe Rc"
    }

    fn evaluate(&self, graph: &KnowledgeGraph, project_root: &Path) -> Result<Vec<Violation>> {
        let rc_pattern = Regex::new(&format!(r"\\b{}<", "Rc")).unwrap();
        let async_indicators = ["async fn", "tokio", "actix", "axum", "hyper"];

        // Collect Rust file paths that are likely async (contain async fn / tokio use)
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
            .collect();
        drop(stmt);
        drop(conn);

        let mut violations = Vec::new();
        for fp in paths {
            let Some(abs_path) = resolve_path(project_root, &fp) else {
                continue;
            };
            let Ok(content) = std::fs::read_to_string(&abs_path) else {
                continue;
            };

            // Only flag files that are async-context (contain async markers)
            let is_async = async_indicators.iter().any(|kw| content.contains(kw));
            if !is_async {
                continue;
            }

            for (i, line) in content.lines().enumerate() {
                let trimmed = line.trim();
                if trimmed.starts_with("//") {
                    continue;
                }
                if rc_pattern.is_match(line) {
                    violations.push(
                        Violation::new(self.id(), self.severity(), &fp,
                            format!("`Rc` on line {} is not Send+Sync — use `Arc` in async code", i + 1))
                            .with_lines((i + 1) as u32, (i + 1) as u32)
                            .with_suggestion("Replace `Rc` with `Arc` and ensure the inner type implements Send+Sync"),
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
    fn concurrency_rule_id() {
        assert_eq!(RcUsageRule.id(), "CONCURRENCY-001");
        assert_eq!(RcUsageRule.severity(), Severity::Error);
    }
}
