// tools/hermes-engine/src/mcp_incidents.rs
//
// TRACK-044: Hermes Incident Ledger & Auto-KB
//
// Five MCP tools that give Hermes a Support-Engineer memory layer:
//   hermes_log_incident    — open a new incident per sub-product
//   hermes_resolve_incident — close it and auto-write a KB article
//   hermes_query_incidents  — list incidents with optional filters
//   hermes_write_kb_article — write a standalone KB article
//   hermes_search_kb        — search KB articles by query

use anyhow::Result;
use chrono::Utc;
use serde_json::json;
use std::path::Path;

use crate::incident_io::*;
use crate::kb_handler::*;
use crate::mcp_memory::{ingest_single_file, slugify};
use crate::HermesEngine;

pub use crate::incident_io::{tool_query_incidents, tool_query_incidents_with_conn};
pub use crate::kb_handler::{tool_search_kb, tool_search_kb_with_conn, tool_write_kb_article};

// ---------------------------------------------------------------------------
// hermes_log_incident
// ---------------------------------------------------------------------------

/// Open a new incident for a sub-product.
/// Creates `memory/incidents/<sub_product>/YYYY-MM-DD_<slug>.md` with OPEN status.
pub fn tool_log_incident(
    engine: &HermesEngine,
    root: &Path,
    args: &serde_json::Value,
) -> Result<String> {
    let sub_product = args["sub_product"].as_str().unwrap_or("unknown");
    let title = args["title"].as_str().unwrap_or("");
    anyhow::ensure!(!title.is_empty(), "hermes_log_incident requires 'title'");

    let severity = args["severity"].as_str().unwrap_or("P2");
    let symptoms = args["symptoms"].as_str().unwrap_or("");
    let tags = str_array(args, "tags");

    let sub_product = validate_sub_product(sub_product);
    let date = Utc::now().format("%Y-%m-%d").to_string();
    let slug = slugify(title);

    let dir = root.join("memory").join("incidents").join(&sub_product);
    std::fs::create_dir_all(&dir)?;

    let filename = format!("{date}_{slug}.md");
    let path = dir.join(&filename);
    anyhow::ensure!(!path.exists(), "incident already exists: {filename}");

    let content = build_incident_md(&date, title, &sub_product, severity, symptoms, &tags, None);
    std::fs::write(&path, &content)?;

    record_incident_event(engine, "incident_opened", title, &path)?;

    if let Err(e) = ingest_single_file(engine, &path) {
        eprintln!("[hermes] ingest after log_incident failed: {e}");
    }

    Ok(serde_json::to_string_pretty(&json!({
        "status": "opened",
        "sub_product": sub_product,
        "slug": slug,
        "severity": severity,
        "path": path.to_string_lossy(),
    }))?)
}

// ---------------------------------------------------------------------------
// hermes_resolve_incident
// ---------------------------------------------------------------------------

/// Resolve an open incident. Updates its file to RESOLVED status and, unless
/// `write_kb` is false, auto-creates a KB article from the resolution details.
pub fn tool_resolve_incident(
    engine: &HermesEngine,
    root: &Path,
    args: &serde_json::Value,
) -> Result<String> {
    let sub_product = args["sub_product"].as_str().unwrap_or("");
    let slug = args["slug"].as_str().unwrap_or("");
    anyhow::ensure!(
        !sub_product.is_empty() && !slug.is_empty(),
        "hermes_resolve_incident requires 'sub_product' and 'slug'"
    );

    let root_cause = args["root_cause"].as_str().unwrap_or("");
    let fix_summary = args["fix_summary"].as_str().unwrap_or("");
    let files_changed = str_array(args, "files_changed");
    let lessons = args["lessons"].as_str().unwrap_or("");
    let write_kb = args["write_kb"].as_bool().unwrap_or(true);

    let sub_product = validate_sub_product(sub_product);
    let dir = root.join("memory").join("incidents").join(&sub_product);

    // Find the incident file (may have a date prefix we don't know)
    let incident_path = find_incident_file(&dir, slug)?;

    // Read existing content to extract title, date, severity, symptoms, tags
    let existing = std::fs::read_to_string(&incident_path)?;
    let (date, title, severity, symptoms, tags) = parse_incident_header(&existing);

    let resolution = ResolutionFields {
        root_cause,
        fix_summary,
        files_changed: &files_changed,
        lessons,
    };
    let content = build_incident_md(
        &date,
        &title,
        &sub_product,
        &severity,
        &symptoms,
        &tags,
        Some(&resolution),
    );
    std::fs::write(&incident_path, &content)?;

    record_incident_event(engine, "incident_resolved", &title, &incident_path)?;

    if let Err(e) = ingest_single_file(engine, &incident_path) {
        eprintln!("[hermes] ingest after resolve_incident failed: {e}");
    }

    let mut kb_path: Option<String> = None;
    if write_kb && !fix_summary.is_empty() {
        let kb_slug = slug.to_string();
        let kb_args = json!({
            "sub_product": sub_product,
            "title": title,
            "problem": symptoms,
            "root_cause": root_cause,
            "solution": fix_summary,
            "related_incidents": [slug],
            "prevention": lessons,
            "tags": tags,
            "slug": kb_slug,
        });
        match tool_write_kb_article(engine, root, &kb_args) {
            Ok(kb_result) => {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&kb_result) {
                    kb_path = v["path"].as_str().map(String::from);
                }
            }
            Err(e) => eprintln!("[hermes] auto-write KB after resolve failed: {e}"),
        }
    }

    Ok(serde_json::to_string_pretty(&json!({
        "status": "resolved",
        "sub_product": sub_product,
        "slug": slug,
        "incident_path": incident_path.to_string_lossy(),
        "kb_path": kb_path,
    }))?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_engine() -> (HermesEngine, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let engine = HermesEngine::in_memory("incident-test").unwrap();
        (engine, dir)
    }

    #[test]
    fn log_incident_creates_file() {
        let (engine, tmp) = make_engine();
        let args = json!({
            "sub_product": "backend",
            "title": "Cosmos auth timeout",
            "severity": "P1",
            "symptoms": "403 errors on all cosmos calls",
            "tags": ["cosmos", "auth"],
        });
        let out = tool_log_incident(&engine, tmp.path(), &args).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["status"], "opened");
        assert_eq!(v["severity"], "P1");

        let path = std::path::Path::new(v["path"].as_str().unwrap());
        assert!(path.exists());
        let content = std::fs::read_to_string(path).unwrap();
        assert!(content.contains("## Status\nOPEN"));
    }

    #[test]
    fn resolve_incident_updates_file_and_creates_kb() {
        let (engine, tmp) = make_engine();
        // First log it
        let log_args = json!({
            "sub_product": "backend",
            "title": "DB timeout fix test",
            "severity": "P2",
            "symptoms": "Timeouts under load",
        });
        let logged = tool_log_incident(&engine, tmp.path(), &log_args).unwrap();
        let lv: serde_json::Value = serde_json::from_str(&logged).unwrap();
        let slug = lv["slug"].as_str().unwrap().to_string();

        // Now resolve it
        let res_args = json!({
            "sub_product": "backend",
            "slug": slug,
            "root_cause": "Connection pool exhaustion",
            "fix_summary": "Increased pool size to 20",
            "files_changed": ["src/db.rs"],
            "lessons": "Monitor pool size via metrics",
            "write_kb": true,
        });
        let out = tool_resolve_incident(&engine, tmp.path(), &res_args).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["status"], "resolved");

        // Incident file should be RESOLVED
        let inc_path = std::path::Path::new(v["incident_path"].as_str().unwrap());
        let inc_content = std::fs::read_to_string(inc_path).unwrap();
        assert!(inc_content.contains("## Status\nRESOLVED"));
        assert!(inc_content.contains("Connection pool exhaustion"));

        // KB article should exist
        assert!(v["kb_path"].is_string());
        let kb_path = std::path::Path::new(v["kb_path"].as_str().unwrap());
        assert!(kb_path.exists());
        let kb_content = std::fs::read_to_string(kb_path).unwrap();
        assert!(kb_content.starts_with("# KB:"));
    }

    #[test]
    fn query_incidents_filters_by_status() {
        let (engine, tmp) = make_engine();
        let log = |title: &str, sp: &str| {
            let args = json!({ "sub_product": sp, "title": title, "severity": "P3" });
            tool_log_incident(&engine, tmp.path(), &args).unwrap();
        };
        log("alpha incident", "backend");
        log("beta incident", "frontend");

        // Resolve alpha
        let lv: serde_json::Value = serde_json::from_str(
            &tool_log_incident(
                &engine,
                tmp.path(),
                &json!({
                    "sub_product": "backend", "title": "gamma incident", "severity": "P1"
                }),
            )
            .unwrap(),
        )
        .unwrap();
        // Actually resolve it
        let slug = lv["slug"].as_str().unwrap().to_string();
        tool_resolve_incident(
            &engine,
            tmp.path(),
            &json!({
                "sub_product": "backend", "slug": slug,
                "root_cause": "bug", "fix_summary": "fixed", "write_kb": false,
            }),
        )
        .unwrap();

        // Query only OPEN incidents
        let out = tool_query_incidents(&engine, tmp.path(), &json!({ "status": "OPEN" })).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        let incidents = v["incidents"].as_array().unwrap();
        assert!(incidents.iter().all(|i| i["status"] == "OPEN"));
    }

    #[test]
    fn validate_sub_product_known_and_unknown() {
        assert_eq!(validate_sub_product("backend"), "backend");
        assert_eq!(
            validate_sub_product("my-custom-svc"),
            "custom/my-custom-svc"
        );
    }
}
