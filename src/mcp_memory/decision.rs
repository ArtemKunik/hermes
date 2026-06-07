// tools/hermes-engine/src/mcp_memory/decision.rs
use anyhow::Result;
use rusqlite::Connection;
use serde_json::json;
use std::path::Path;

use crate::mcp_memory::utils::{slugify, str_array};
use crate::{accounting::Accountant, HermesEngine};

pub fn tool_write_decision(
    engine: &HermesEngine,
    root: &Path,
    args: &serde_json::Value,
    conn: &Connection,
) -> Result<String> {
    let title = args["title"].as_str().unwrap_or("");
    anyhow::ensure!(!title.is_empty(), "hermes_write_decision requires 'title'");

    let status = args["status"].as_str().unwrap_or("OPEN");
    let context = args["context"].as_str().unwrap_or("");
    let root_cause = args["root_cause"].as_str().unwrap_or("");
    let what_worked = str_array(args, "what_worked");
    let what_failed = str_array(args, "what_failed");
    let next_steps = str_array(args, "next_steps");
    let related_files = str_array(args, "related_files");
    let tags = str_array(args, "tags");
    let slug = args["slug"]
        .as_str()
        .map(String::from)
        .unwrap_or_else(|| slugify(title));

    let dec_dir = root.join("memory").join("decisions");
    std::fs::create_dir_all(&dec_dir)?;
    let path = dec_dir.join(format!("{slug}.md"));
    let existed = path.exists();

    let content = build_decision_md(
        title,
        status,
        context,
        &what_worked,
        &what_failed,
        root_cause,
        &next_steps,
        &related_files,
        &tags,
    );
    std::fs::write(&path, &content)?;

    let acct = Accountant::from_conn(conn, engine.project_id(), engine.session_id());
    let tags_csv = if tags.is_empty() {
        None
    } else {
        Some(tags.join(", "))
    };
    acct.record_memory_event(
        if existed {
            "decision_updated"
        } else {
            "decision_created"
        },
        Some(title),
        Some(&path.to_string_lossy()),
        tags_csv.as_deref(),
    )?;

    // Ingest in a background thread to avoid re-acquiring the db mutex
    // that the actor thread already holds (same pattern as tool_remember).
    let db = engine.db().clone();
    let project_id = engine.project_id().to_string();
    let ingest_path = path.clone();
    std::thread::spawn(move || {
        let graph = crate::graph::KnowledgeGraph::new(db, &project_id);
        let pipeline = crate::ingestion::IngestionPipeline::new(&graph);
        let env_acc = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        if let Err(e) = pipeline.ingest_file(&ingest_path, &env_acc) {
            eprintln!("[hermes] background ingest after write_decision failed: {e}");
        }
    });

    Ok(serde_json::to_string_pretty(&json!({
        "status":     if existed { "updated" } else { "created" },
        "path":       path.to_string_lossy(),
        "slug":       slug,
        "size_bytes": content.len(),
    }))?)
}

#[allow(clippy::too_many_arguments)]
fn build_decision_md(
    title: &str,
    status: &str,
    context: &str,
    what_worked: &[String],
    what_failed: &[String],
    root_cause: &str,
    next_steps: &[String],
    related_files: &[String],
    tags: &[String],
) -> String {
    let mut md = format!("# Decision: {title}\n\n## Status\n{status}\n\n");
    if !context.is_empty() {
        md.push_str(&format!("## Context\n{context}\n\n"));
    }
    if !what_worked.is_empty() {
        md.push_str("## What Was Tried\n");
        for (i, w) in what_worked.iter().enumerate() {
            md.push_str(&format!("### {}. {}\n\n", i + 1, w));
        }
    }
    if !what_failed.is_empty() {
        md.push_str("## What Didn't Work\n");
        for f in what_failed {
            md.push_str(&format!("- {f}\n"));
        }
        md.push('\n');
    }
    if !root_cause.is_empty() {
        md.push_str(&format!("## Root Cause (Probable)\n{root_cause}\n\n"));
    }
    if !next_steps.is_empty() {
        md.push_str("## Next Steps (Untried)\n");
        for (i, s) in next_steps.iter().enumerate() {
            md.push_str(&format!("{}. {s}\n", i + 1));
        }
        md.push('\n');
    }
    if !related_files.is_empty() {
        md.push_str("## Related Files\n");
        for f in related_files {
            md.push_str(&format!("- {f}\n"));
        }
        md.push('\n');
    }
    if !tags.is_empty() {
        md.push_str(&format!("## Tags\n{}\n", tags.join(", ")));
    }
    md
}
