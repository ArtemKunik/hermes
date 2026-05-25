// tools/hermes-engine/src/arch_rules/mod.rs
// TRACK-045: Architecture rule trait, violation type, and default rule registry.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::graph::KnowledgeGraph;

pub mod classifier;
pub mod concurrency_rules;
pub mod fintech_rules;
pub mod layer_rules;
pub mod query_rules;
pub mod safety_rules;
pub mod size_rules;

// ---------------------------------------------------------------------------
// Severity
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

impl Severity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Info => "info",
        }
    }

    pub fn parse_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "error" => Self::Error,
            "warning" => Self::Warning,
            _ => Self::Info,
        }
    }
}

// ---------------------------------------------------------------------------
// Violation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Violation {
    pub rule_id: String,
    pub severity: String,
    pub file_path: String,
    pub line_start: Option<u32>,
    pub line_end: Option<u32>,
    pub symbol: Option<String>,
    pub message: String,
    pub suggestion: Option<String>,
}

impl Violation {
    pub fn new(
        rule_id: &str,
        severity: Severity,
        file_path: &str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            rule_id: rule_id.to_string(),
            severity: severity.as_str().to_string(),
            file_path: file_path.to_string(),
            line_start: None,
            line_end: None,
            symbol: None,
            message: message.into(),
            suggestion: None,
        }
    }

    pub fn with_lines(mut self, start: u32, end: u32) -> Self {
        self.line_start = Some(start);
        self.line_end = Some(end);
        self
    }

    pub fn with_symbol(mut self, symbol: &str) -> Self {
        self.symbol = Some(symbol.to_string());
        self
    }

    pub fn with_suggestion(mut self, suggestion: &str) -> Self {
        self.suggestion = Some(suggestion.to_string());
        self
    }
}

// ---------------------------------------------------------------------------
// ArchRule trait
// ---------------------------------------------------------------------------

pub trait ArchRule: Send + Sync {
    fn id(&self) -> &str;
    fn severity(&self) -> Severity;
    fn description(&self) -> &str;
    fn evaluate(&self, graph: &KnowledgeGraph, project_root: &Path) -> Result<Vec<Violation>>;
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// Build the default rule registry (all 26 rules).
pub fn default_rules() -> Vec<Box<dyn ArchRule>> {
    vec![
        Box::new(layer_rules::LayerHandlerImportsStore),
        Box::new(layer_rules::LayerHandlerBusinessLogic),
        Box::new(layer_rules::LayerServiceImportsHandler),
        Box::new(layer_rules::LayerComponentFetch),
        Box::new(layer_rules::LayerComponentImportsApi),
        Box::new(size_rules::FileSizeRule),
        Box::new(size_rules::MethodSizeRule),
        Box::new(safety_rules::UnwrapInProdRule),
        Box::new(safety_rules::ExpectInProdRule),
        Box::new(safety_rules::AnyWithoutSafetyRule),
        Box::new(concurrency_rules::RcUsageRule),
        Box::new(query_rules::StringInterpolationQueryRule),
        Box::new(fintech_rules::NoF64MoneyRule),
        Box::new(fintech_rules::RawDecimalRule),
        Box::new(fintech_rules::StringlyMoneyRule),
        Box::new(fintech_rules::MutableTransactionRule),
        Box::new(fintech_rules::MissingCloneRule),
        Box::new(fintech_rules::RcDomainRule),
        Box::new(fintech_rules::UncheckedArithmeticRule),
        Box::new(fintech_rules::DecimalOverflowRule),
        Box::new(fintech_rules::MissingSerdeRule),
        Box::new(fintech_rules::MissingValidationRule),
        Box::new(fintech_rules::MissingAuditSpanRule),
        Box::new(fintech_rules::MissingTraceIdRule),
        Box::new(fintech_rules::CurrencyOpsRule),
        Box::new(fintech_rules::PanicDomainRule),
    ]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_ordering() {
        assert!(Severity::Error > Severity::Warning);
        assert!(Severity::Warning > Severity::Info);
    }

    #[test]
    fn severity_parse_roundtrip() {
        assert_eq!(Severity::parse_str("error"), Severity::Error);
        assert_eq!(Severity::parse_str("warning"), Severity::Warning);
        assert_eq!(Severity::parse_str("info"), Severity::Info);
        assert_eq!(Severity::parse_str("unknown"), Severity::Info);
    }

    #[test]
    fn default_rules_has_twenty_six_entries() {
        assert_eq!(default_rules().len(), 26);
    }

    #[test]
    fn violation_builder() {
        let v = Violation::new("TEST-001", Severity::Error, "src/main.rs", "test")
            .with_lines(1, 10)
            .with_symbol("foo")
            .with_suggestion("fix it");
        assert_eq!(v.severity, "error");
        assert_eq!(v.line_start, Some(1));
        assert!(v.symbol.is_some());
        assert!(v.suggestion.is_some());
    }
}
