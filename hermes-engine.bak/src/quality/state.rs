// tools/hermes-engine/src/quality/state.rs
// TRACK-049: Persistent quality state — Finding/ModuleState/QualityState structs
// and atomic load/save to .hermes/quality-state.json.

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub const SCHEMA_VERSION: u32 = 1;

/// Snapshot of arch-lint violations used as the drift baseline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LintBaseline {
    pub recorded_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    pub total: usize,
    pub by_rule: HashMap<String, usize>,
    /// Fingerprints: "rule_id:relative_file:line" for precise new-violation detection.
    pub violation_keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FindingStatus {
    Open,
    Resolved,
    Wontfix,
}

impl FindingStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Resolved => "resolved",
            Self::Wontfix => "wontfix",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub id: String,
    pub dim: String,
    pub tier: String,
    pub zone: String,
    pub file: String,
    pub line_hint: Option<u32>,
    pub description: String,
    pub evidence: String,
    pub status: FindingStatus,
    pub detected_ts: String,
    pub resolved_ts: Option<String>,
    pub wontfix_reason: Option<String>,
}

impl Finding {
    pub fn new(
        dim: impl Into<String>,
        tier: impl Into<String>,
        zone: impl Into<String>,
        file: impl Into<String>,
        line_hint: Option<u32>,
        description: impl Into<String>,
        evidence: impl Into<String>,
    ) -> Self {
        let short = &Uuid::new_v4().to_string().replace('-', "")[..8];
        Self {
            id: format!("Q-{}", short.to_uppercase()),
            dim: dim.into(),
            tier: tier.into(),
            zone: zone.into(),
            file: file.into(),
            line_hint,
            description: description.into(),
            evidence: evidence.into(),
            status: FindingStatus::Open,
            detected_ts: Utc::now().to_rfc3339(),
            resolved_ts: None,
            wontfix_reason: None,
        }
    }

    /// Check whether this finding is a duplicate of `other` (same dim + file + evidence prefix).
    pub fn is_duplicate_of(&self, other: &Finding) -> bool {
        self.dim == other.dim
            && self.file == other.file
            && self.evidence.len() >= 20
            && other.evidence.starts_with(&self.evidence[..20.min(self.evidence.len())])
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleState {
    pub score: f64,
    pub score_prev: f64,
    pub scan_ts: String,
    pub findings: Vec<Finding>,
}

impl Default for ModuleState {
    fn default() -> Self {
        Self {
            score: 100.0,      // no findings yet → perfect score
            score_prev: 100.0,
            scan_ts: String::new(),
            findings: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityState {
    pub schema_version: u32,
    pub last_scan: String,
    pub modules: HashMap<String, ModuleState>,
    pub project_score: f64,
    pub project_score_prev: f64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub lint_baseline: Option<LintBaseline>,
}

impl Default for QualityState {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            last_scan: Utc::now().to_rfc3339(),
            modules: HashMap::new(),
            project_score: 100.0,
            project_score_prev: 100.0,
            lint_baseline: None,
        }
    }
}

/// Canonical path for quality-state.json inside the project.
pub fn state_path(project_root: &Path) -> PathBuf {
    project_root.join(".hermes").join("quality-state.json")
}

/// Load quality state from disk, returning a fresh default if not present.
pub fn load(project_root: &Path) -> Result<QualityState> {
    let path = state_path(project_root);
    if !path.exists() {
        return Ok(QualityState::default());
    }
    let data = fs::read_to_string(&path)?;
    Ok(serde_json::from_str(&data)?)
}

/// Atomically save quality state (write to .tmp, then rename).
pub fn save(project_root: &Path, state: &QualityState) -> Result<()> {
    let dir = project_root.join(".hermes");
    fs::create_dir_all(&dir)?;
    let path = state_path(project_root);
    let tmp = path.with_extension("json.tmp");
    let data = serde_json::to_string_pretty(state)?;
    fs::write(&tmp, &data)?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_finding_new_generates_prefixed_id() {
        let f = Finding::new("QD-01", "T4", "production", "src/main.rs", Some(10), "desc", "evidence fragment");
        assert!(f.id.starts_with("Q-"), "id should start with Q-: {}", f.id);
        assert_eq!(f.status, FindingStatus::Open);
        assert_eq!(f.dim, "QD-01");
    }

    #[test]
    fn test_load_returns_default_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let state = load(dir.path()).unwrap();
        assert_eq!(state.schema_version, SCHEMA_VERSION);
        assert!(state.modules.is_empty());
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = QualityState::default();
        state.project_score = 87.5;
        save(dir.path(), &state).unwrap();
        let loaded = load(dir.path()).unwrap();
        assert!((loaded.project_score - 87.5).abs() < f64::EPSILON);
    }
}
