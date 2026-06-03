use anyhow::{bail, Result};
use chrono::Utc;
use reqwest::Client;
use serde_json::{json, Value};
use shared_rust::llm_gateway_client::LlmGatewayClient;
use std::path::Path;

use crate::quality::{
    review::{all_dimensions, enumerate_files, review_file_dimension},
    score::{compute_module_score, compute_project_score, finding_priority, suspect_guard},
    state::{load, save, Finding, FindingStatus, QualityState},
};
use crate::HermesEngine;

const DEFAULT_GATEWAY: &str = "http://localhost:3001";

fn gateway_client() -> LlmGatewayClient {
    LlmGatewayClient::from_env(Client::new()).unwrap_or_else(|| {
        LlmGatewayClient::new(Client::new(), DEFAULT_GATEWAY.to_string(), None)
    })
}

/// Derive module name from a file path (crate directory name under ChartApp/).
/// Works for both relative (ChartApp/foo/...) and absolute paths.
fn module_name(file: &str) -> String {
    let norm = file.replace('\\', "/");
    if let Some(pos) = norm.find("ChartApp/") {
        let after = &norm[pos + "ChartApp/".len()..];
        let seg = after.split('/').next().unwrap_or("unknown");
        if !seg.is_empty() {
            return seg.to_string();
        }
    }
    // Fallback: last meaningful directory segment
    let parts: Vec<&str> = norm
        .split('/')
        .filter(|s| !s.is_empty() && !s.contains(':'))
        .collect();
    parts.first().copied().unwrap_or("unknown").to_string()
}

/// Run LLM-guided review over files in `path`, optionally filtered by `dim` and `tier`.
pub fn tool_quality_review(
    _engine: &HermesEngine,
    project_root: &Path,
    args: &Value,
) -> Result<String> {
    let scan_path_str = args["path"].as_str().unwrap_or("ChartApp");
    let dim_filter = args["dim"].as_str();
    let tier_filter = args["tier"].as_str();
    let scan_path = project_root.join(scan_path_str);
    anyhow::ensure!(
        scan_path.exists(),
        "path not found: {}",
        scan_path.display()
    );

    let mut state = load(project_root)?;
    state.last_scan = Utc::now().to_rfc3339();

    let dims: Vec<&crate::quality::review::Dimension> = all_dimensions()
        .iter()
        .filter(|d| dim_filter.map_or(true, |f| d.id == f))
        .filter(|d| tier_filter.map_or(true, |f| d.tier == f))
        .collect();

    let verbose = args["verbose"].as_bool().unwrap_or(false);
    if verbose {
        eprintln!(
            "[hermes-quality] starting review path={} dims={} tier={} verbose=true",
            scan_path.display(),
            dims.len(),
            tier_filter.unwrap_or("<any>")
        );
    }

    let files = enumerate_files(&scan_path);
    let client = gateway_client();
    let mut files_scanned = 0u64;
    let mut findings_added = 0u64;
    let mut findings_detected: Vec<Finding> = Vec::new();
    let mut gateway_errors = 0u64;
    let mut first_error: Option<String> = None;

    for (file_path, zone) in &files {
        let Ok(content) = std::fs::read_to_string(file_path) else {
            continue;
        };
        let file_str = file_path.to_string_lossy().replace('\\', "/");
        let module = module_name(&file_str);
        let module_state = state.modules.entry(module.clone()).or_default();

        for dim in &dims {
            if verbose {
                eprintln!(
                    "[hermes-quality] scanning {} zone={:?} dim={}",
                    file_str, zone, dim.id
                );
            }
            match review_file_dimension(&client, file_path, &content, zone, dim) {
                Ok(new_findings) => {
                    findings_detected.extend(new_findings.iter().cloned());
                    for nf in new_findings {
                        if !module_state
                            .findings
                            .iter()
                            .any(|ef| ef.is_duplicate_of(&nf))
                        {
                            module_state.findings.push(nf);
                            findings_added += 1;
                        }
                    }
                }
                Err(e) => {
                    gateway_errors += 1;
                    if first_error.is_none() {
                        first_error = Some(e.to_string());
                        eprintln!("[hermes-quality] all providers failed: {e}");
                    }
                }
            }
        }
        files_scanned += 1;
        // Recompute score after each file to keep state consistent
        let prev = module_state.score;
        module_state.score_prev = prev;
        module_state.score = compute_module_score(&module_state.findings);
        module_state.scan_ts = state.last_scan.clone();

        let force_accept = args["force_accept"].as_bool().unwrap_or(false);
        if suspect_guard(prev, module_state.score, &module_state.findings) && !force_accept {
            bail!("[hermes-quality] SUSPECT GUARD: module {module} jumped {:.1} pts — add force_accept: true to bypass", (prev - module_state.score).abs());
        }
    }

    let prev_project = state.project_score;
    state.project_score_prev = prev_project;
    state.project_score = compute_project_score(&state.modules);
    save(project_root, &state)?;

    if verbose {
        eprintln!(
            "[hermes-quality] finished review files_scanned={} findings_added={} gateway_errors={} project_score={:.1} delta={:.1}",
            files_scanned,
            findings_added,
            gateway_errors,
            state.project_score,
            state.project_score - prev_project
        );
    }

    Ok(serde_json::to_string_pretty(&json!({
        "files_scanned": files_scanned,
        "findings_detected": findings_detected.len(),
        "findings_added": findings_added,
        "findings": findings_detected,
        "gateway_errors": gateway_errors,
        "project_score": state.project_score,
        "project_score_delta": state.project_score - prev_project,
        "dims_checked": dims.len(),
    }))?)
}

pub fn tool_quality_score(
    _engine: &HermesEngine,
    project_root: &Path,
    args: &Value,
) -> Result<String> {
    let module_filter = args["module"].as_str();
    let show_trend = args["trend"].as_bool().unwrap_or(false);
    let state = load(project_root)?;

    let mut rows: Vec<Value> = state
        .modules
        .iter()
        .filter(|(name, _)| module_filter.map_or(true, |f| name.as_str() == f))
        .map(|(name, ms)| {
            let open_t4 = ms
                .findings
                .iter()
                .filter(|f| f.status == FindingStatus::Open && f.tier == "T4")
                .count();
            let open_t3 = ms
                .findings
                .iter()
                .filter(|f| f.status == FindingStatus::Open && f.tier == "T3")
                .count();
            let mut row = json!({
                "module": name,
                "score": ms.score,
                "open_t4": open_t4,
                "open_t3": open_t3,
                "scan_ts": ms.scan_ts,
            });
            if show_trend {
                row["score_prev"] = json!(ms.score_prev);
                row["delta"] = json!(ms.score - ms.score_prev);
            }
            row
        })
        .collect();

    rows.sort_by(|a, b| {
        a["score"]
            .as_f64()
            .unwrap_or(100.0)
            .partial_cmp(&b["score"].as_f64().unwrap_or(100.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(serde_json::to_string_pretty(&json!({
        "project_score": state.project_score,
        "modules": rows,
        "last_scan": state.last_scan,
    }))?)
}

pub fn tool_quality_next(
    _engine: &HermesEngine,
    project_root: &Path,
    args: &Value,
) -> Result<String> {
    let module_filter = args["module"].as_str();
    let state = load(project_root)?;

    let top: Option<(&str, &Finding)> = state
        .modules
        .iter()
        .filter(|(name, _)| module_filter.map_or(true, |f| name.as_str() == f))
        .flat_map(|(name, ms)| {
            ms.findings
                .iter()
                .filter(|f| f.status == FindingStatus::Open)
                .map(move |f| (name.as_str(), f))
        })
        .max_by(|(_, a), (_, b)| {
            finding_priority(a)
                .partial_cmp(&finding_priority(b))
                .unwrap_or(std::cmp::Ordering::Equal)
        });

    match top {
        None => Ok(json!({"message": "No open findings."}).to_string()),
        Some((module, f)) => Ok(serde_json::to_string_pretty(&json!({
            "id": f.id,
            "module": module,
            "file": f.file,
            "line_hint": f.line_hint,
            "dim": f.dim,
            "tier": f.tier,
            "description": f.description,
            "evidence": f.evidence,
            "priority": finding_priority(f),
            "resolve_command": format!("hermes resolve-review --id {}", f.id),
        }))?),
    }
}

pub fn tool_quality_resolve(
    _engine: &HermesEngine,
    project_root: &Path,
    args: &Value,
) -> Result<String> {
    let id = args["id"].as_str().unwrap_or("");
    anyhow::ensure!(!id.is_empty(), "id is required");

    let mut state = load(project_root)?;
    let resolved = apply_to_finding(&mut state, id, |f| {
        f.status = FindingStatus::Resolved;
        f.resolved_ts = Some(Utc::now().to_rfc3339());
    })?;
    anyhow::ensure!(resolved, "finding {id} not found");

    state.recompute_scores();
    save(project_root, &state)?;

    Ok(json!({"resolved": id, "project_score": state.project_score}).to_string())
}

pub fn tool_quality_wontfix(
    _engine: &HermesEngine,
    project_root: &Path,
    args: &Value,
) -> Result<String> {
    let id = args["id"].as_str().unwrap_or("");
    let reason = args["reason"].as_str().unwrap_or("").trim().to_string();
    anyhow::ensure!(!id.is_empty(), "id is required");
    anyhow::ensure!(!reason.is_empty(), "reason is required");

    let mut state = load(project_root)?;
    let found = apply_to_finding(&mut state, id, |f| {
        f.status = FindingStatus::Wontfix;
        f.wontfix_reason = Some(reason.clone());
    })?;
    anyhow::ensure!(found, "finding {id} not found");

    state.recompute_scores();
    save(project_root, &state)?;

    Ok(json!({"wontfix": id, "reason": reason, "project_score": state.project_score}).to_string())
}

fn apply_to_finding(state: &mut QualityState, id: &str, f: impl Fn(&mut Finding)) -> Result<bool> {
    for ms in state.modules.values_mut() {
        if let Some(finding) = ms.findings.iter_mut().find(|ff| ff.id == id) {
            f(finding);
            return Ok(true);
        }
    }
    Ok(false)
}
