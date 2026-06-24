// tools/hermes-engine/src/incident_io.rs
use anyhow::Result;
use std::path::{Path, PathBuf};
use serde_json::json;
use crate::accounting::Accountant;
use crate::HermesEngine;
use crate::graph_types::NodeType;

pub const KNOWN_SUB_PRODUCTS: &[&str] = &[
    "chartapp-server",
    "hermes-engine",
    "llm-gateway",
    "telegram-gateway",
    "trainer-worker",
    "doctor-service",
    "watchdog",
    "codex-worker",
    "mastermind-daemon",
    "chartapp-client",
    "android-app",
    "backend",
    "frontend",
    "infra",
];

pub struct ResolutionFields<'a> {
    pub root_cause: &'a str,
    pub fix_summary: &'a str,
    pub files_changed: &'a [String],
    pub lessons: &'a str,
}

pub fn validate_sub_product(sp: &str) -> String {
    if KNOWN_SUB_PRODUCTS.contains(&sp) {
        sp.to_string()
    } else {
        format!("custom/{}", sp)
    }
}

pub fn find_incident_file(dir: &Path, slug: &str) -> Result<PathBuf> {
    if !dir.exists() {
        anyhow::bail!("incident directory not found: {}", dir.display());
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if name.contains(slug) && name.ends_with(".md") {
            return Ok(path);
        }
    }
    anyhow::bail!("incident not found with slug: {slug}");
}

pub fn parse_incident_header(content: &str) -> (String, String, String, String, Vec<String>) {
    let mut date = String::new();
    let mut title = String::new();
    let mut severity = "P2".to_string();
    let mut symptoms = String::new();
    let mut tags = Vec::new();

    let mut in_symptoms = false;

    for line in content.lines() {
        if line.starts_with("# Incident: ") {
            title = line["# Incident: ".len()..].to_string();
        } else if line.starts_with("- date: ") {
            date = line["- date: ".len()..].to_string();
        } else if line.starts_with("- severity: ") {
            severity = line["- severity: ".len()..].to_string();
        } else if line.starts_with("- tags: ") {
            tags = line["- tags: ".len()..]
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        } else if line.starts_with("## Symptoms") {
            in_symptoms = true;
        } else if line.starts_with("## ") && in_symptoms {
            in_symptoms = false;
        } else if in_symptoms {
            if !line.trim().is_empty() {
                if !symptoms.is_empty() {
                    symptoms.push('\n');
                }
                symptoms.push_str(line);
            }
        }
    }

    (date, title, severity, symptoms, tags)
}

pub fn build_incident_md(
    date: &str,
    title: &str,
    sub_product: &str,
    severity: &str,
    symptoms: &str,
    tags: &[String],
    resolution: Option<&ResolutionFields>,
) -> String {
    let mut md = format!(
        "# Incident: {title}\n\n\
         ## Metadata\n\
         - date: {date}\n\
         - sub_product: {sub_product}\n\
         - severity: {severity}\n\
         - tags: {tags}\n\n\
         ## Status\n\
         {status}\n\n\
         ## Symptoms\n\
         {symptoms}\n",
        tags = tags.join(", "),
        status = if resolution.is_some() { "RESOLVED" } else { "OPEN" }
    );

    if let Some(res) = resolution {
        md.push_str(&format!(
            "\n## Root Cause\n{rc}\n\n\
             ## Resolution\n{fix}\n\n\
             ## Files Changed\n{files}\n\n\
             ## Lessons Learned\n{lessons}\n",
            rc = res.root_cause,
            fix = res.fix_summary,
            files = res
                .files_changed
                .iter()
                .map(|f| format!("- {f}"))
                .collect::<Vec<_>>()
                .join("\n"),
            lessons = res.lessons
        ));
    }

    md
}

pub fn extract_field(content: &str, field: &str) -> Option<String> {
    let prefix = format!("## {field}");
    let mut found = false;
    let mut result = String::new();
    for line in content.lines() {
        if line.starts_with(&prefix) {
            found = true;
            continue;
        }
        if found && line.starts_with("## ") {
            break;
        }
        if found && !line.trim().is_empty() {
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(line.trim());
        }
    }
    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

pub fn str_array(args: &serde_json::Value, key: &str) -> Vec<String> {
    args[key]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

pub fn record_incident_event(
    engine: &crate::HermesEngine,
    event_type: &str,
    title: &str,
    path: &Path,
) -> Result<()> {
    let acct = Accountant::new(engine.db().clone(), engine.project_id(), engine.session_id());
    acct.record_memory_event(event_type, Some(title), Some(&path.to_string_lossy()), None)?;
    Ok(())
}

/// List incidents, optionally filtered by sub_product, status, and severity.
pub fn tool_query_incidents(
    engine: &HermesEngine,
    root: &Path,
    args: &serde_json::Value,
) -> Result<String> {
    let db = engine.read_db().lock().unwrap_or_else(|e| e.into_inner());
    tool_query_incidents_with_conn(engine, &db, root, args)
}

pub fn tool_query_incidents_with_conn(
    engine: &HermesEngine,
    conn: &rusqlite::Connection,
    root: &Path,
    args: &serde_json::Value,
) -> Result<String> {
    let filter_sp = args["sub_product"].as_str();
    let filter_status = args["status"].as_str(); // "OPEN" | "RESOLVED"
    let filter_severity = args["severity"].as_str(); // "P0".."P3"

    let incidents_root = root.join("memory").join("incidents");
    if !incidents_root.exists() {
        return Ok(serde_json::to_string_pretty(&json!({ "incidents": [] }))?);
    }

    let mut results: Vec<serde_json::Value> = Vec::new();

    // Walk sub-product directories
    for sp_entry in std::fs::read_dir(&incidents_root)? {
        let sp_entry = sp_entry?;
        if !sp_entry.file_type()?.is_dir() {
            continue;
        }
        let sp_name = sp_entry.file_name().to_string_lossy().to_string();
        if let Some(fp) = filter_sp {
            if sp_name != fp && sp_name != validate_sub_product(fp) {
                continue;
            }
        }

        for file_entry in std::fs::read_dir(sp_entry.path())? {
            let file_entry = file_entry?;
            let file_path = file_entry.path();
            if file_path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let content = std::fs::read_to_string(&file_path).unwrap_or_default();
            let (date, title, severity, _symptoms, _tags) = parse_incident_header(&content);
            let status = extract_field(&content, "Status").unwrap_or_else(|| "OPEN".to_string());

            if let Some(fs) = filter_status {
                if !status.eq_ignore_ascii_case(fs) {
                    continue;
                }
            }
            if let Some(fsev) = filter_severity {
                if !severity.eq_ignore_ascii_case(fsev) {
                    continue;
                }
            }

            let filename = file_path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            // slug = everything after the date prefix "YYYY-MM-DD_"
            let slug = if filename.len() > 11 {
                filename[11..].to_string()
            } else {
                filename.clone()
            };

            results.push(json!({
                "sub_product": sp_name,
                "slug": slug,
                "title": title,
                "severity": severity,
                "status": status,
                "date": date,
                "path": file_path.to_string_lossy(),
            }));
        }
    }

    // Sort by date descending (newest first)
    results.sort_by(|a, b| {
        b["date"]
            .as_str()
            .unwrap_or("")
            .cmp(a["date"].as_str().unwrap_or(""))
    });

    let acct = Accountant::from_conn(conn, engine.project_id(), engine.session_id());
    let _ = acct.record_query("query_incidents", 100, 0, 1000);

    Ok(serde_json::to_string_pretty(&json!({ "incidents": results }))?)
}
