// tools/hermes-engine/src/quality/score.rs
// TRACK-049: Quality scoring formulae and suspect guard logic.

use std::collections::HashMap;

use crate::quality::state::{Finding, FindingStatus, ModuleState};

/// Severity tier weight per spec: T4=4×, T3=2×, T2=1×, T1=0.5×.
pub fn tier_weight(tier: &str) -> f64 {
    match tier {
        "T4" => 4.0,
        "T3" => 2.0,
        "T2" => 1.0,
        "T1" => 0.5,
        _ => 1.0,
    }
}

/// Zone multiplier for scoring — read from the stored zone string on each finding.
pub fn zone_multiplier_from_str(zone: &str) -> f64 {
    match zone {
        "production" => 1.0,
        "test" => 0.25,
        "config" | "script" => 0.1,
        _ => 0.0,
    }
}

/// Compute a single module's score from its findings.
///
/// Formula (spec §Architecture/Scoring Formula):
///   raw = 100 - Σ(tier_weight × zone_multiplier) for open findings
///   wontfix_penalty = Σ(0.5 × tier_weight × zone_multiplier) for wontfix findings
///   score = clamp(raw - wontfix_penalty, 0, 100)
pub fn compute_module_score(findings: &[Finding]) -> f64 {
    let open_penalty: f64 = findings
        .iter()
        .filter(|f| f.status == FindingStatus::Open)
        .map(|f| tier_weight(&f.tier) * zone_multiplier_from_str(&f.zone))
        .sum();

    let wontfix_penalty: f64 = findings
        .iter()
        .filter(|f| f.status == FindingStatus::Wontfix)
        .map(|f| 0.5 * tier_weight(&f.tier) * zone_multiplier_from_str(&f.zone))
        .sum();

    (100.0 - open_penalty - wontfix_penalty).clamp(0.0, 100.0)
}

/// Compute project-wide weighted average score (equal module weight; no LOC data needed).
pub fn compute_project_score(modules: &HashMap<String, ModuleState>) -> f64 {
    if modules.is_empty() {
        return 100.0;
    }
    let sum: f64 = modules.values().map(|m| m.score).sum();
    sum / modules.len() as f64
}

/// Suspect guard: returns true (should warn) if the module score increased ≥ 20 points
/// in one scan without any finding being resolved in the new batch.
///
/// This prevents phantom improvements from stale state or erroneous LLM responses.
pub fn suspect_guard(prev_score: f64, new_score: f64, new_findings: &[Finding]) -> bool {
    let delta = new_score - prev_score;
    if delta < 20.0 {
        return false;
    }
    let has_resolves = new_findings
        .iter()
        .any(|f| f.status == FindingStatus::Resolved);
    !has_resolves
}

/// Priority of a finding for `next-review` ordering: higher = more urgent.
pub fn finding_priority(finding: &Finding) -> f64 {
    tier_weight(&finding.tier) * zone_multiplier_from_str(&finding.zone)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quality::state::Finding;

    fn open_finding(tier: &str, zone: &str) -> Finding {
        Finding::new("QD-01", tier, zone, "file.rs", None, "desc", "evidence fragment one")
    }

    fn wontfix_finding(tier: &str, zone: &str) -> Finding {
        let mut f = Finding::new("QD-01", tier, zone, "file.rs", None, "desc", "evidence fragment two");
        f.status = FindingStatus::Wontfix;
        f
    }

    #[test]
    fn test_tier_weight_values() {
        assert!((tier_weight("T4") - 4.0).abs() < f64::EPSILON);
        assert!((tier_weight("T3") - 2.0).abs() < f64::EPSILON);
        assert!((tier_weight("T2") - 1.0).abs() < f64::EPSILON);
        assert!((tier_weight("T1") - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_compute_module_score_no_findings() {
        assert!((compute_module_score(&[]) - 100.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_compute_module_score_open_t4_production() {
        // One open T4 production finding: 100 - (4.0 × 1.0) = 96.0
        let findings = vec![open_finding("T4", "production")];
        let score = compute_module_score(&findings);
        assert!((score - 96.0).abs() < 0.01, "expected 96.0, got {score}");
    }

    #[test]
    fn test_compute_module_score_wontfix_halved() {
        // One wontfix T4 production: penalty = 0.5 × 4.0 × 1.0 = 2.0 → score = 98.0
        let findings = vec![wontfix_finding("T4", "production")];
        let score = compute_module_score(&findings);
        assert!((score - 98.0).abs() < 0.01, "expected 98.0, got {score}");
    }

    #[test]
    fn test_suspect_guard_fires_on_large_jump_without_resolves() {
        let new_findings = vec![open_finding("T2", "production")];
        assert!(suspect_guard(60.0, 90.0, &new_findings));
    }

    #[test]
    fn test_suspect_guard_bypassed_when_resolves_present() {
        let mut resolved = open_finding("T4", "production");
        resolved.status = FindingStatus::Resolved;
        assert!(!suspect_guard(60.0, 90.0, &[resolved]));
    }
}
