use anyhow::Result;
use regex::Regex;
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

fn is_test_file(file_path: &str) -> bool {
    let norm = file_path.replace('\\', "/").to_lowercase();
    norm.contains("/tests/")
        || norm.ends_with("_test.rs")
        || norm.ends_with("_tests.rs")
        || norm.ends_with(".test.ts")
        || norm.ends_with(".spec.ts")
}

fn is_domain_file(file_path: &str) -> bool {
    let norm = file_path.replace('\\', "/").to_lowercase();
    norm.contains("/domain/") || norm.contains("/model/")
}

fn get_rust_files(graph: &KnowledgeGraph) -> Result<Vec<String>> {
    let conn = graph.db().lock().map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut stmt = conn.prepare(
        "SELECT DISTINCT file_path FROM nodes
         WHERE project_id = ?1
           AND node_type = 'file'
           AND file_path LIKE '%.rs'
           AND file_path NOT LIKE '%node_modules%'",
    )?;
    let paths: Vec<String> = stmt
        .query_map(rusqlite::params![graph.project_id()], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();
    drop(stmt);
    drop(conn);
    Ok(paths)
}

fn get_all_files(graph: &KnowledgeGraph) -> Result<Vec<String>> {
    let conn = graph.db().lock().map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut stmt = conn.prepare(
        "SELECT DISTINCT file_path FROM nodes
         WHERE project_id = ?1
           AND node_type = 'file'
           AND file_path NOT LIKE '%node_modules%'",
    )?;
    let paths: Vec<String> = stmt
        .query_map(rusqlite::params![graph.project_id()], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();
    drop(stmt);
    drop(conn);
    Ok(paths)
}

fn read_file_content(abs_path: &Path) -> Option<String> {
    std::fs::read_to_string(abs_path).ok()
}

fn is_comment_or_string_line(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with("//") || trimmed.starts_with('#')
}

// ---------------------------------------------------------------------------
// FIN-001: No f64/f32 for monetary fields
// ---------------------------------------------------------------------------

pub struct NoF64MoneyRule;

impl ArchRule for NoF64MoneyRule {
    fn id(&self) -> &str {
        "FIN-001"
    }
    fn severity(&self) -> Severity {
        Severity::Error
    }
    fn description(&self) -> &str {
        "Financial values use f64/f32 — use rust_decimal::Decimal instead"
    }

    fn evaluate(&self, graph: &KnowledgeGraph, project_root: &Path) -> Result<Vec<Violation>> {
        let pattern = Regex::new(
            r"\b(amount|price|balance|money|principal|interest|total|fee|rate|cost|value|premium|payment|salary|commission|dividend)\s*:\s*(f64|f32)\b",
        )?;
        let files = get_rust_files(graph)?;
        let mut violations = Vec::new();
        for fp in files {
            if is_test_file(&fp) {
                continue;
            }
            let Some(abs_path) = resolve_path(project_root, &fp) else {
                continue;
            };
            let Some(content) = read_file_content(&abs_path) else {
                continue;
            };
            for (i, line) in content.lines().enumerate() {
                if is_comment_or_string_line(line) {
                    continue;
                }
                if pattern.is_match(line) {
                    violations.push(
                        Violation::new(self.id(), self.severity(), &fp,
                            format!("Monetary field uses f64/f32 on line {} — use Decimal from rust_decimal", i + 1))
                            .with_lines((i + 1) as u32, (i + 1) as u32)
                            .with_suggestion("Replace with `Decimal` from the rust_decimal crate"),
                    );
                }
            }
        }
        Ok(violations)
    }
}

// ---------------------------------------------------------------------------
// FIN-002: No raw Decimal in struct fields (must use newtype wrapper)
// ---------------------------------------------------------------------------

pub struct RawDecimalRule;

impl ArchRule for RawDecimalRule {
    fn id(&self) -> &str {
        "FIN-002"
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }
    fn description(&self) -> &str {
        "Bare Decimal used as struct field type — use a newtype wrapper like Amount(Decimal)"
    }

    fn evaluate(&self, graph: &KnowledgeGraph, project_root: &Path) -> Result<Vec<Violation>> {
        let pattern = Regex::new(r":\s*Decimal\b")?;
        let newtype_pattern = Regex::new(r"struct\s+\w+\s*\(\s*Decimal\b")?;
        let files = get_rust_files(graph)?;
        let mut violations = Vec::new();
        for fp in files {
            if is_test_file(&fp) {
                continue;
            }
            let Some(abs_path) = resolve_path(project_root, &fp) else {
                continue;
            };
            let Some(content) = read_file_content(&abs_path) else {
                continue;
            };
            let lines: Vec<&str> = content.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                if is_comment_or_string_line(line) {
                    continue;
                }
                if newtype_pattern.is_match(line) {
                    continue;
                }
                if pattern.is_match(line) {
                    violations.push(
                        Violation::new(
                            self.id(),
                            self.severity(),
                            &fp,
                            format!(
                                "Bare Decimal field type on line {} — wrap in a newtype",
                                i + 1
                            ),
                        )
                        .with_lines((i + 1) as u32, (i + 1) as u32)
                        .with_suggestion(
                            "Create a newtype: `struct Amount(Decimal);` and use `Amount` instead",
                        ),
                    );
                }
            }
        }
        Ok(violations)
    }
}

// ---------------------------------------------------------------------------
// FIN-003: No String/&str for monetary values
// ---------------------------------------------------------------------------

pub struct StringlyMoneyRule;

impl ArchRule for StringlyMoneyRule {
    fn id(&self) -> &str {
        "FIN-003"
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }
    fn description(&self) -> &str {
        "Monetary value stored as String/&str — use a typed Amount newtype"
    }

    fn evaluate(&self, graph: &KnowledgeGraph, project_root: &Path) -> Result<Vec<Violation>> {
        let pattern = Regex::new(
            r"\b(amount|price|balance|money|principal|interest|total|fee|rate|cost|value|premium|payment)\s*:\s*(String|&str)\b",
        )?;
        let files = get_rust_files(graph)?;
        let mut violations = Vec::new();
        for fp in files {
            if is_test_file(&fp) {
                continue;
            }
            let Some(abs_path) = resolve_path(project_root, &fp) else {
                continue;
            };
            let Some(content) = read_file_content(&abs_path) else {
                continue;
            };
            for (i, line) in content.lines().enumerate() {
                if is_comment_or_string_line(line) {
                    continue;
                }
                if pattern.is_match(line) {
                    violations.push(
                        Violation::new(
                            self.id(),
                            self.severity(),
                            &fp,
                            format!(
                                "Monetary value as String/&str on line {} — use Amount newtype",
                                i + 1
                            ),
                        )
                        .with_lines((i + 1) as u32, (i + 1) as u32)
                        .with_suggestion("Replace with `Amount` (or similar newtype over Decimal)"),
                    );
                }
            }
        }
        Ok(violations)
    }
}

// ---------------------------------------------------------------------------
// FIN-004: No &mut self on transaction/ledger types
// ---------------------------------------------------------------------------

pub struct MutableTransactionRule;

impl ArchRule for MutableTransactionRule {
    fn id(&self) -> &str {
        "FIN-004"
    }
    fn severity(&self) -> Severity {
        Severity::Error
    }
    fn description(&self) -> &str {
        "Transaction/Ledger types must not have &mut self methods — audit trail requires immutability"
    }

    fn evaluate(&self, graph: &KnowledgeGraph, project_root: &Path) -> Result<Vec<Violation>> {
        let trans_pattern = Regex::new(r"\b(transaction|ledger|journal|account|entry|posting)\b")?;
        let mut_pattern = Regex::new(r"fn\s+\w+\s*\(\s*&\s*mut\s+self\b")?;
        let files = get_rust_files(graph)?;
        let mut violations = Vec::new();
        for fp in files {
            if is_test_file(&fp) {
                continue;
            }
            let Some(abs_path) = resolve_path(project_root, &fp) else {
                continue;
            };
            let Some(content) = read_file_content(&abs_path) else {
                continue;
            };
            if !trans_pattern.is_match(&content) {
                continue;
            }
            for (i, line) in content.lines().enumerate() {
                if is_comment_or_string_line(line) {
                    continue;
                }
                if mut_pattern.is_match(line) {
                    violations.push(
                        Violation::new(self.id(), self.severity(), &fp,
                            format!("&mut self on line {} in type with transaction semantics — use immutable pattern", i + 1))
                            .with_lines((i + 1) as u32, (i + 1) as u32)
                            .with_suggestion("Use `&self` with interior mutability (RefCell/Mutex) or event sourcing pattern"),
                    );
                }
            }
        }
        Ok(violations)
    }
}

// ---------------------------------------------------------------------------
// FIN-005: Domain value objects missing Clone
// ---------------------------------------------------------------------------

pub struct MissingCloneRule;

impl ArchRule for MissingCloneRule {
    fn id(&self) -> &str {
        "FIN-005"
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }
    fn description(&self) -> &str {
        "Domain value object (Decimal newtype) missing Clone derive — breaks event sourcing"
    }

    fn evaluate(&self, graph: &KnowledgeGraph, project_root: &Path) -> Result<Vec<Violation>> {
        let newtype_pattern =
            Regex::new(r"(?m)^\s*(pub\s+)?struct\s+(\w+)\s*\(\s*(pub\s+)?Decimal\b")?;
        let clone_pattern = Regex::new(r"Clone")?;
        let files = get_rust_files(graph)?;
        let mut violations = Vec::new();
        for fp in files {
            if is_test_file(&fp) {
                continue;
            }
            let Some(abs_path) = resolve_path(project_root, &fp) else {
                continue;
            };
            let Some(content) = read_file_content(&abs_path) else {
                continue;
            };
            let lines: Vec<&str> = content.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                if let Some(caps) = newtype_pattern.captures(line) {
                    let struct_name = caps.get(2).map(|m| m.as_str()).unwrap_or("");
                    let has_derive = (i >= 1 && clone_pattern.is_match(lines[i - 1]))
                        || (i >= 2 && clone_pattern.is_match(lines[i - 2]))
                        || (i >= 3 && clone_pattern.is_match(lines[i - 3]))
                        || (i >= 4 && clone_pattern.is_match(lines[i - 4]));
                    if !has_derive {
                        violations.push(
                            Violation::new(
                                self.id(),
                                self.severity(),
                                &fp,
                                format!(
                                    "`{}` wraps Decimal but missing Clone derive on line {}",
                                    struct_name,
                                    i + 1
                                ),
                            )
                            .with_lines((i + 1) as u32, (i + 1) as u32)
                            .with_suggestion(&format!(
                                "Add `#[derive(Clone)]` to `{}`",
                                struct_name
                            )),
                        );
                    }
                }
            }
        }
        Ok(violations)
    }
}

// ---------------------------------------------------------------------------
// FIN-006: Rc used in domain/model files (should use Arc)
// ---------------------------------------------------------------------------

pub struct RcDomainRule;

impl ArchRule for RcDomainRule {
    fn id(&self) -> &str {
        "FIN-006"
    }
    fn severity(&self) -> Severity {
        Severity::Error
    }
    fn description(&self) -> &str {
        "Rc used in domain code — use Arc for thread-safe sharing of domain objects"
    }

    fn evaluate(&self, graph: &KnowledgeGraph, project_root: &Path) -> Result<Vec<Violation>> {
        let rc_pattern = Regex::new(r"\bRc<")?;
        let files = get_rust_files(graph)?;
        let mut violations = Vec::new();
        for fp in files {
            if is_test_file(&fp) {
                continue;
            }
            if !is_domain_file(&fp) {
                continue;
            }
            let Some(abs_path) = resolve_path(project_root, &fp) else {
                continue;
            };
            let Some(content) = read_file_content(&abs_path) else {
                continue;
            };
            for (i, line) in content.lines().enumerate() {
                if is_comment_or_string_line(line) {
                    continue;
                }
                if rc_pattern.is_match(line) {
                    violations.push(
                        Violation::new(
                            self.id(),
                            self.severity(),
                            &fp,
                            format!("Rc used on line {} in domain code — use Arc", i + 1),
                        )
                        .with_lines((i + 1) as u32, (i + 1) as u32)
                        .with_suggestion(
                            "Replace `Rc` with `Arc` and ensure inner type is Send+Sync",
                        ),
                    );
                }
            }
        }
        Ok(violations)
    }
}

// ---------------------------------------------------------------------------
// FIN-007: Unchecked arithmetic — unwrap/expect on Decimal ops
// ---------------------------------------------------------------------------

pub struct UncheckedArithmeticRule;

impl ArchRule for UncheckedArithmeticRule {
    fn id(&self) -> &str {
        "FIN-007"
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }
    fn description(&self) -> &str {
        "Unwrap/expect on Decimal arithmetic — use proper error propagation"
    }

    fn evaluate(&self, graph: &KnowledgeGraph, project_root: &Path) -> Result<Vec<Violation>> {
        let pattern = Regex::new(
            r"\.(add|sub|mul|div|checked_add|checked_sub|checked_mul|checked_div)\s*\([^)]*\)\s*\.(unwrap|expect)\s*\(",
        )?;
        let files = get_rust_files(graph)?;
        let mut violations = Vec::new();
        for fp in files {
            if is_test_file(&fp) {
                continue;
            }
            let Some(abs_path) = resolve_path(project_root, &fp) else {
                continue;
            };
            let Some(content) = read_file_content(&abs_path) else {
                continue;
            };
            for (i, line) in content.lines().enumerate() {
                if is_comment_or_string_line(line) {
                    continue;
                }
                if pattern.is_match(line) {
                    violations.push(
                        Violation::new(self.id(), self.severity(), &fp,
                            format!("Unwrap/expect on Decimal arithmetic on line {} — errors can lose money", i + 1))
                            .with_lines((i + 1) as u32, (i + 1) as u32)
                            .with_suggestion("Propagate the Result with `?` instead of unwrapping"),
                    );
                }
            }
        }
        Ok(violations)
    }
}

// ---------------------------------------------------------------------------
// FIN-008: Decimal unchecked arithmetic via +/- operator
// ---------------------------------------------------------------------------

pub struct DecimalOverflowRule;

impl ArchRule for DecimalOverflowRule {
    fn id(&self) -> &str {
        "FIN-008"
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }
    fn description(&self) -> &str {
        "Decimal values use +/- operator — prefer checked methods (add/sub) to prevent silent overflow"
    }

    fn evaluate(&self, graph: &KnowledgeGraph, project_root: &Path) -> Result<Vec<Violation>> {
        let plus_pattern = Regex::new(r"\bDecimal\s*\+\s*Decimal\b")?;
        let minus_pattern = Regex::new(r"\bDecimal\s*-\s*Decimal\b")?;
        let files = get_rust_files(graph)?;
        let mut violations = Vec::new();
        for fp in files {
            if is_test_file(&fp) {
                continue;
            }
            let Some(abs_path) = resolve_path(project_root, &fp) else {
                continue;
            };
            let Some(content) = read_file_content(&abs_path) else {
                continue;
            };
            for (i, line) in content.lines().enumerate() {
                if is_comment_or_string_line(line) {
                    continue;
                }
                if plus_pattern.is_match(line) || minus_pattern.is_match(line) {
                    violations.push(
                        Violation::new(self.id(), self.severity(), &fp,
                            format!("Decimal +/- operator on line {} — use checked `.add()`/`.sub()` to handle overflow", i + 1))
                            .with_lines((i + 1) as u32, (i + 1) as u32)
                            .with_suggestion("Use `amount.add(other)?` instead of `amount + other`"),
                    );
                }
            }
        }
        Ok(violations)
    }
}

// ---------------------------------------------------------------------------
// FIN-009: Domain types missing serde derives
// ---------------------------------------------------------------------------

pub struct MissingSerdeRule;

impl ArchRule for MissingSerdeRule {
    fn id(&self) -> &str {
        "FIN-009"
    }
    fn severity(&self) -> Severity {
        Severity::Info
    }
    fn description(&self) -> &str {
        "Domain type missing Serialize/Deserialize derives — needed for persistence and serialization"
    }

    fn evaluate(&self, graph: &KnowledgeGraph, project_root: &Path) -> Result<Vec<Violation>> {
        let struct_pattern = Regex::new(r"(?m)^\s*(pub\s+)?struct\s+(\w+)")?;
        let serde_pattern = Regex::new(r"(Serialize|Deserialize)")?;
        let money_type_pattern = Regex::new(
            r"\b(Amount|Price|Balance|Money|Principal|Interest|Fee|Rate|Cost|Payment)\b",
        )?;
        let files = get_rust_files(graph)?;
        let mut violations = Vec::new();
        for fp in files {
            if is_test_file(&fp) {
                continue;
            }
            let Some(abs_path) = resolve_path(project_root, &fp) else {
                continue;
            };
            let Some(content) = read_file_content(&abs_path) else {
                continue;
            };
            let lines: Vec<&str> = content.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                if let Some(caps) = struct_pattern.captures(line) {
                    let struct_name = caps.get(2).map(|m| m.as_str()).unwrap_or("");
                    if !money_type_pattern.is_match(struct_name) {
                        continue;
                    }
                    let has_serde = (i >= 1 && serde_pattern.is_match(lines[i - 1]))
                        || (i >= 2 && serde_pattern.is_match(lines[i - 2]))
                        || (i >= 3 && serde_pattern.is_match(lines[i - 3]))
                        || (i >= 4 && serde_pattern.is_match(lines[i - 4]));
                    if !has_serde {
                        violations.push(
                            Violation::new(
                                self.id(),
                                self.severity(),
                                &fp,
                                format!(
                                    "`{}` on line {} missing serde derives",
                                    struct_name,
                                    i + 1
                                ),
                            )
                            .with_lines((i + 1) as u32, (i + 1) as u32)
                            .with_suggestion(&format!(
                                "Add `#[derive(Serialize, Deserialize)]` to `{}`",
                                struct_name
                            )),
                        );
                    }
                }
            }
        }
        Ok(violations)
    }
}

// ---------------------------------------------------------------------------
// FIN-010: Decimal newtype without validation
// ---------------------------------------------------------------------------

pub struct MissingValidationRule;

impl ArchRule for MissingValidationRule {
    fn id(&self) -> &str {
        "FIN-010"
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }
    fn description(&self) -> &str {
        "Decimal newtype missing validation — monetary values should be validated on construction"
    }

    fn evaluate(&self, graph: &KnowledgeGraph, project_root: &Path) -> Result<Vec<Violation>> {
        let newtype_pattern = Regex::new(
            r"(?m)^\s*(pub\s+)?struct\s+(Amount|Price|Balance|Money|Principal|Interest|Fee|Rate|Cost|Payment)\s*\(\s*(pub\s+)?Decimal\b",
        )?;
        let files = get_rust_files(graph)?;
        let mut violations = Vec::new();
        for fp in files {
            if is_test_file(&fp) {
                continue;
            }
            let Some(abs_path) = resolve_path(project_root, &fp) else {
                continue;
            };
            let Some(content) = read_file_content(&abs_path) else {
                continue;
            };
            let lines: Vec<&str> = content.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                if let Some(caps) = newtype_pattern.captures(line) {
                    let struct_name = caps.get(2).map(|m| m.as_str()).unwrap_or("");
                    let has_new_fn = content.contains(&format!("fn new("))
                        || content.contains("fn try_new(")
                        || content.contains("fn from_str(")
                        || content.contains("fn validate(");
                    if !has_new_fn {
                        violations.push(
                            Violation::new(self.id(), self.severity(), &fp,
                                format!("`{}(Decimal)` on line {} has no validation fn", struct_name, i + 1))
                                .with_lines((i + 1) as u32, (i + 1) as u32)
                                .with_suggestion(&format!("Add a `pub fn new(value: Decimal) -> Result<Self, Error>` with validation checks")),
                        );
                    }
                }
            }
        }
        Ok(violations)
    }
}

// ---------------------------------------------------------------------------
// FIN-011: fn apply() method missing tracing/audit span
// ---------------------------------------------------------------------------

pub struct MissingAuditSpanRule;

impl ArchRule for MissingAuditSpanRule {
    fn id(&self) -> &str {
        "FIN-011"
    }
    fn severity(&self) -> Severity {
        Severity::Info
    }
    fn description(&self) -> &str {
        "Transaction apply() method missing tracing span — audit trail requires structured logging"
    }

    fn evaluate(&self, graph: &KnowledgeGraph, project_root: &Path) -> Result<Vec<Violation>> {
        let apply_pattern = Regex::new(r"fn\s+apply\s*\(")?;
        let span_pattern = Regex::new(
            r"(span!|trace_span|info_span|debug_span|#[instrument]|audit_log|trace_id)",
        )?;
        let files = get_rust_files(graph)?;
        let mut violations = Vec::new();
        for fp in files {
            if is_test_file(&fp) {
                continue;
            }
            let Some(abs_path) = resolve_path(project_root, &fp) else {
                continue;
            };
            let Some(content) = read_file_content(&abs_path) else {
                continue;
            };
            if !apply_pattern.is_match(&content) {
                continue;
            }
            if span_pattern.is_match(&content) {
                continue;
            }
            for (i, line) in content.lines().enumerate() {
                if apply_pattern.is_match(line) && !is_comment_or_string_line(line) {
                    violations.push(
                        Violation::new(self.id(), self.severity(), &fp,
                            format!("apply() method on line {} without tracing span — audit trail incomplete", i + 1))
                            .with_lines((i + 1) as u32, (i + 1) as u32)
                            .with_suggestion("Add `#[instrument]` or `let span = info_span!(\"apply\");`"),
                    );
                }
            }
        }
        Ok(violations)
    }
}

// ---------------------------------------------------------------------------
// FIN-012: Domain event/transaction struct missing trace_id field
// ---------------------------------------------------------------------------

pub struct MissingTraceIdRule;

impl ArchRule for MissingTraceIdRule {
    fn id(&self) -> &str {
        "FIN-012"
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }
    fn description(&self) -> &str {
        "Domain event/transaction struct missing trace_id: Uuid — required for audit trail correlation"
    }

    fn evaluate(&self, graph: &KnowledgeGraph, project_root: &Path) -> Result<Vec<Violation>> {
        let struct_pattern =
            Regex::new(r"(?m)^\s*(pub\s+)?struct\s+(\w+(Event|Command|Transaction))\b")?;
        let trace_pattern = Regex::new(r"\btrace_id\b")?;
        let files = get_rust_files(graph)?;
        let mut violations = Vec::new();
        for fp in files {
            if is_test_file(&fp) {
                continue;
            }
            let Some(abs_path) = resolve_path(project_root, &fp) else {
                continue;
            };
            let Some(content) = read_file_content(&abs_path) else {
                continue;
            };
            if !trace_pattern.is_match(&content) {
                let lines: Vec<&str> = content.lines().collect();
                for (i, line) in lines.iter().enumerate() {
                    if let Some(caps) = struct_pattern.captures(line) {
                        let struct_name = caps.get(2).map(|m| m.as_str()).unwrap_or("");
                        violations.push(
                            Violation::new(
                                self.id(),
                                self.severity(),
                                &fp,
                                format!(
                                    "`{}` on line {} missing trace_id field",
                                    struct_name,
                                    i + 1
                                ),
                            )
                            .with_lines((i + 1) as u32, (i + 1) as u32)
                            .with_suggestion(&format!(
                                "Add `trace_id: Uuid` field to `{}`",
                                struct_name
                            )),
                        );
                    }
                }
            }
        }
        Ok(violations)
    }
}

// ---------------------------------------------------------------------------
// FIN-013: Amount::add/sub without currency match check
// ---------------------------------------------------------------------------

pub struct CurrencyOpsRule;

impl ArchRule for CurrencyOpsRule {
    fn id(&self) -> &str {
        "FIN-013"
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }
    fn description(&self) -> &str {
        "Amount arithmetic without currency match check — can silently mix currencies"
    }

    fn evaluate(&self, graph: &KnowledgeGraph, project_root: &Path) -> Result<Vec<Violation>> {
        let amount_op_pattern = Regex::new(r"Amount::(add|sub|mul|div)\s*\(")?;
        let currency_check_pattern = Regex::new(
            r"(currency|currency_eq|same_currency|ensure_same_currency|CurrencyMismatch)",
        )?;
        let files = get_rust_files(graph)?;
        let mut violations = Vec::new();
        for fp in files {
            if is_test_file(&fp) {
                continue;
            }
            let Some(abs_path) = resolve_path(project_root, &fp) else {
                continue;
            };
            let Some(content) = read_file_content(&abs_path) else {
                continue;
            };
            if !amount_op_pattern.is_match(&content) {
                continue;
            }
            if !currency_check_pattern.is_match(&content) {
                for (i, line) in content.lines().enumerate() {
                    if amount_op_pattern.is_match(line) && !is_comment_or_string_line(line) {
                        violations.push(
                            Violation::new(self.id(), self.severity(), &fp,
                                format!("Amount::add/sub on line {} without currency match check — may mix currencies", i + 1))
                                .with_lines((i + 1) as u32, (i + 1) as u32)
                                .with_suggestion("Add a currency equality check before the operation (e.g., `ensure_same_currency` pattern)"),
                        );
                        break;
                    }
                }
            }
        }
        Ok(violations)
    }
}

// ---------------------------------------------------------------------------
// FIN-014: No panic/unreachable/todo in domain code
// ---------------------------------------------------------------------------

pub struct PanicDomainRule;

impl ArchRule for PanicDomainRule {
    fn id(&self) -> &str {
        "FIN-014"
    }
    fn severity(&self) -> Severity {
        Severity::Error
    }
    fn description(&self) -> &str {
        "panic!/unreachable!/todo! in domain/model code — use proper error handling"
    }

    fn evaluate(&self, graph: &KnowledgeGraph, project_root: &Path) -> Result<Vec<Violation>> {
        let panic_pattern = Regex::new(r"\b(panic!|unreachable!|todo!|unimplemented!)\b")?;
        let files = get_all_files(graph)?;
        let mut violations = Vec::new();
        for fp in files {
            if is_test_file(&fp) {
                continue;
            }
            if !is_domain_file(&fp) {
                continue;
            }
            let Some(abs_path) = resolve_path(project_root, &fp) else {
                continue;
            };
            let Some(content) = read_file_content(&abs_path) else {
                continue;
            };
            for (i, line) in content.lines().enumerate() {
                if is_comment_or_string_line(line) {
                    continue;
                }
                if panic_pattern.is_match(line) {
                    violations.push(
                        Violation::new(self.id(), self.severity(), &fp,
                            format!("panic! on line {} in domain code — use Result for fallible operations", i + 1))
                            .with_lines((i + 1) as u32, (i + 1) as u32)
                            .with_suggestion("Return a `Result` with a domain error type instead of panicking"),
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
    fn no_f64_money_rule_id() {
        assert_eq!(NoF64MoneyRule.id(), "FIN-001");
        assert_eq!(NoF64MoneyRule.severity(), Severity::Error);
    }

    #[test]
    fn raw_decimal_rule_id() {
        assert_eq!(RawDecimalRule.id(), "FIN-002");
        assert_eq!(RawDecimalRule.severity(), Severity::Warning);
    }

    #[test]
    fn stringly_money_rule_id() {
        assert_eq!(StringlyMoneyRule.id(), "FIN-003");
        assert_eq!(StringlyMoneyRule.severity(), Severity::Warning);
    }

    #[test]
    fn mutable_transaction_rule_id() {
        assert_eq!(MutableTransactionRule.id(), "FIN-004");
        assert_eq!(MutableTransactionRule.severity(), Severity::Error);
    }

    #[test]
    fn missing_clone_rule_id() {
        assert_eq!(MissingCloneRule.id(), "FIN-005");
        assert_eq!(MissingCloneRule.severity(), Severity::Warning);
    }

    #[test]
    fn rc_domain_rule_id() {
        assert_eq!(RcDomainRule.id(), "FIN-006");
        assert_eq!(RcDomainRule.severity(), Severity::Error);
    }

    #[test]
    fn unchecked_arithmetic_rule_id() {
        assert_eq!(UncheckedArithmeticRule.id(), "FIN-007");
        assert_eq!(UncheckedArithmeticRule.severity(), Severity::Warning);
    }

    #[test]
    fn decimal_overflow_rule_id() {
        assert_eq!(DecimalOverflowRule.id(), "FIN-008");
        assert_eq!(DecimalOverflowRule.severity(), Severity::Warning);
    }

    #[test]
    fn missing_serde_rule_id() {
        assert_eq!(MissingSerdeRule.id(), "FIN-009");
        assert_eq!(MissingSerdeRule.severity(), Severity::Info);
    }

    #[test]
    fn missing_validation_rule_id() {
        assert_eq!(MissingValidationRule.id(), "FIN-010");
        assert_eq!(MissingValidationRule.severity(), Severity::Warning);
    }

    #[test]
    fn missing_audit_span_rule_id() {
        assert_eq!(MissingAuditSpanRule.id(), "FIN-011");
        assert_eq!(MissingAuditSpanRule.severity(), Severity::Info);
    }

    #[test]
    fn missing_trace_id_rule_id() {
        assert_eq!(MissingTraceIdRule.id(), "FIN-012");
        assert_eq!(MissingTraceIdRule.severity(), Severity::Warning);
    }

    #[test]
    fn currency_ops_rule_id() {
        assert_eq!(CurrencyOpsRule.id(), "FIN-013");
        assert_eq!(CurrencyOpsRule.severity(), Severity::Warning);
    }

    #[test]
    fn panic_domain_rule_id() {
        assert_eq!(PanicDomainRule.id(), "FIN-014");
        assert_eq!(PanicDomainRule.severity(), Severity::Error);
    }

    #[test]
    fn is_domain_file_detection() {
        assert!(is_domain_file("src/domain/account.rs"));
        assert!(is_domain_file("src/model/transaction.rs"));
        assert!(!is_domain_file("src/handlers/mod.rs"));
    }

    #[test]
    fn is_test_file_detection() {
        assert!(is_test_file("src/tests/mod.rs"));
        assert!(is_test_file("src/foo_test.rs"));
        assert!(!is_test_file("src/domain/account.rs"));
    }
}
