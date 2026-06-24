// tools/hermes-engine/src/arch_rules/safety_rules.rs
// TRACK-045: SAFETY-001 (unwrap in prod), SAFETY-002 (expect in prod), SAFETY-003 (any without comment).

use anyhow::Result;
use regex::Regex;
use std::path::Path;

use crate::arch_rules::{ArchRule, Severity, Violation};
use crate::graph::KnowledgeGraph;

// ---------------------------------------------------------------------------
// Shared: scan file content for a pattern, return line numbers of matches.
// ---------------------------------------------------------------------------

fn scan_file_for_pattern(path: &Path, pattern: &Regex) -> Vec<u32> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return vec![];
    };
    content
        .lines()
        .enumerate()
        .filter(|(_, line)| {
            let trimmed = line.trim();
            // Skip pure comment lines
            !trimmed.starts_with("//") && !trimmed.starts_with('#') && pattern.is_match(line)
        })
        .map(|(i, _)| (i + 1) as u32)
        .collect()
}

fn resolve_path(project_root: &Path, file_path: &str) -> Option<std::path::PathBuf> {
    let p = std::path::Path::new(file_path);
    if p.is_absolute() && p.exists() {
        return Some(p.to_path_buf());
    }
    let joined = project_root.join(file_path);
    if joined.exists() {
        return Some(joined);
    }
    None
}

fn is_test_file(file_path: &str) -> bool {
    let norm = file_path.replace('\\', "/").to_lowercase();
    norm.contains("/tests/")
        || norm.ends_with("_test.rs")
        || norm.ends_with("_tests.rs")
        || norm.ends_with(".test.ts")
        || norm.ends_with(".spec.ts")
}

// ---------------------------------------------------------------------------
// SAFETY-001: unwrap() in non-test production Rust code
// ---------------------------------------------------------------------------

pub struct UnwrapInProdRule;

impl ArchRule for UnwrapInProdRule {
    fn id(&self) -> &str { "SAFETY-001" }
    fn severity(&self) -> Severity { Severity::Warning }
    fn description(&self) -> &str {
        "unwrap() used in non-test Rust production code — prefer ? or expect with message"
    }

    fn evaluate(&self, graph: &KnowledgeGraph, project_root: &Path) -> Result<Vec<Violation>> {
        let pattern = Regex::new(r"\.unwrap\(\)").expect("invalid regex");
        rust_content_scan(graph, project_root, self.id(), self.severity(), &pattern,
            "unwrap() panics on None/Err — use ? or explicit error handling",
            "Replace .unwrap() with ? or .unwrap_or_else(|e| panic!(\"context: {e}\"))")
    }
}

// ---------------------------------------------------------------------------
// SAFETY-002: expect() in non-test production Rust code
// ---------------------------------------------------------------------------

pub struct ExpectInProdRule;

impl ArchRule for ExpectInProdRule {
    fn id(&self) -> &str { "SAFETY-002" }
    fn severity(&self) -> Severity { Severity::Warning }
    fn description(&self) -> &str {
        "expect() used in non-test Rust production code — prefer ? propagation"
    }

    fn evaluate(&self, graph: &KnowledgeGraph, project_root: &Path) -> Result<Vec<Violation>> {
        let pattern = Regex::new(r"\.expect\(").unwrap();
        rust_content_scan(graph, project_root, self.id(), self.severity(), &pattern,
            "expect() panics in production — use ? to propagate errors",
            "Replace .expect(...) with ? and add context via .context(\"...\")")
    }
}

fn rust_content_scan(
    graph: &KnowledgeGraph,
    project_root: &Path,
    rule_id: &str,
    severity: Severity,
    pattern: &Regex,
    message: &str,
    suggestion: &str,
) -> Result<Vec<Violation>> {
    let conn = graph.db().lock().map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut stmt = conn.prepare(
        "SELECT DISTINCT file_path FROM nodes
         WHERE project_id = ?1
           AND node_type = 'file'
           AND (file_path LIKE '%.rs')",
    )?;
    let paths: Vec<String> = stmt
        .query_map(rusqlite::params![graph.project_id()], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .filter(|fp: &String| !is_test_file(fp))
        .collect();
    drop(stmt);
    drop(conn);

    let mut violations = Vec::new();
    for fp in paths {
        if let Some(abs_path) = resolve_path(project_root, &fp) {
            for line_num in scan_file_for_pattern(&abs_path, pattern) {
                violations.push(
                    Violation::new(rule_id, severity.clone(), &fp, message)
                        .with_lines(line_num, line_num)
                        .with_suggestion(suggestion),
                );
            }
        }
    }
    Ok(violations)
}

// ---------------------------------------------------------------------------
// SAFETY-003: TypeScript `any` without `// SAFETY:` comment
// ---------------------------------------------------------------------------

pub struct AnyWithoutSafetyRule;

impl ArchRule for AnyWithoutSafetyRule {
    fn id(&self) -> &str { "SAFETY-003" }
    fn severity(&self) -> Severity { Severity::Warning }
    fn description(&self) -> &str {
        "TypeScript `any` type used without a preceding `// SAFETY:` comment"
    }

    fn evaluate(&self, graph: &KnowledgeGraph, project_root: &Path) -> Result<Vec<Violation>> {
        let any_pattern = Regex::new(r":\s*any\b|as\s+any\b").unwrap();
        let safety_pattern = Regex::new(r"//\s*SAFETY:").unwrap();

        let conn = graph.db().lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut stmt = conn.prepare(
            "SELECT DISTINCT file_path FROM nodes
             WHERE project_id = ?1
               AND node_type = 'file'
               AND (file_path LIKE '%.ts' OR file_path LIKE '%.tsx')",
        )?;
        let paths: Vec<String> = stmt
            .query_map(rusqlite::params![graph.project_id()], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .filter(|fp: &String| !is_test_file(fp))
            .collect();
        drop(stmt);
        drop(conn);

        let mut violations = Vec::new();
        for fp in paths {
            let Some(abs_path) = resolve_path(project_root, &fp) else { continue };
            let Ok(content) = std::fs::read_to_string(&abs_path) else { continue };
            let lines: Vec<&str> = content.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                if any_pattern.is_match(line) {
                    // Check if the preceding line has a SAFETY comment
                    let prev_has_safety = i > 0 && safety_pattern.is_match(lines[i - 1]);
                    let same_line_safety = safety_pattern.is_match(line);
                    if !prev_has_safety && !same_line_safety {
                        violations.push(
                            Violation::new(self.id(), self.severity(), &fp,
                                format!("`any` type on line {} without // SAFETY: comment", i + 1))
                                .with_lines((i + 1) as u32, (i + 1) as u32)
                                .with_suggestion("Add `// SAFETY: <reason>` on the preceding line, or replace `any` with a specific type"),
                        );
                    }
                }
            }
        }
        Ok(violations)
    }
}
