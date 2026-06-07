// tools/hermes-engine/src/mcp_memory/session.rs
use anyhow::Result;
use chrono::Utc;
use rusqlite::Connection;
use serde_json::json;
use std::path::{Path, PathBuf};

use crate::mcp_memory::utils::{build_md, ingest_single_file, slugify, str_array};
use crate::{accounting::Accountant, HermesEngine};
// ingest_single_file is still used by write_session_checkpoint below

pub fn tool_remember(
    engine: &HermesEngine,
    root: &Path,
    args: &serde_json::Value,
    conn: &Connection,
) -> Result<String> {
    let topic = args["topic"].as_str().unwrap_or("untitled-session");
    let summary = args["summary"].as_str().unwrap_or("");
    let tags = str_array(args, "tags");
    let files = str_array(args, "files_touched");
    let decisions = str_array(args, "decisions");
    let problems = str_array(args, "problems");
    let actions = str_array(args, "actions");

    let mut metrics: Vec<(String, u64)> = Vec::new();
    if let Some(obj) = args["metrics"].as_object() {
        for (k, v) in obj {
            if let Some(n) = v.as_u64() {
                metrics.push((k.clone(), n));
            }
        }
    }

    anyhow::ensure!(!summary.is_empty(), "hermes_remember requires 'summary'");

    let date = Utc::now().format("%Y-%m-%d").to_string();
    let slug = slugify(topic);
    let mem_dir = root.join("memory").join("sessions");
    std::fs::create_dir_all(&mem_dir)?;

    let mut filename = format!("{date}_{slug}.md");
    let mut path = mem_dir.join(&filename);
    let mut ctr = 1u32;
    while path.exists() {
        ctr += 1;
        filename = format!("{date}_{slug}_{ctr}.md");
        path = mem_dir.join(&filename);
    }

    let content = build_md(
        &date, topic, &tags, &files, summary, &decisions, &problems, &actions, &metrics,
    );
    std::fs::write(&path, &content)?;

    let acct = Accountant::from_conn(conn, engine.project_id(), engine.session_id());
    let tags_csv = if tags.is_empty() {
        None
    } else {
        Some(tags.join(", "))
    };
    acct.record_memory_event(
        "session_saved",
        Some(topic),
        Some(&path.to_string_lossy()),
        tags_csv.as_deref(),
    )?;

    for (k, v) in &metrics {
        let val_str = v.to_string();
        acct.record_memory_event("metric", Some(k), None, Some(&val_str))?;
    }

    // Ingest in a background thread so tool_remember returns well within the
    // 15 s MCP client timeout. The file is already durably written; searchability
    // follows asynchronously. (Previously this ran synchronously and reliably
    // exceeded the timeout for large session summaries.)
    let db = engine.db().clone();
    let project_id = engine.project_id().to_string();
    let ingest_path = path.clone();
    std::thread::spawn(move || {
        let graph = crate::graph::KnowledgeGraph::new(db, &project_id);
        let pipeline = crate::ingestion::IngestionPipeline::new(&graph);
        let env_acc = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        if let Err(e) = pipeline.ingest_file(&ingest_path, &env_acc) {
            eprintln!("[hermes] background ingest after remember failed: {e}");
        }
    });

    Ok(serde_json::to_string_pretty(&json!({
        "status": "saved",
        "path": path.to_string_lossy(),
        "size_bytes": content.len(),
        "indexed": "pending",
    }))?)
}

pub fn tool_memory_stats(engine: &HermesEngine) -> Result<String> {
    let diagnostic_db = engine.diagnostic_db()?;
    let conn = diagnostic_db
        .as_ref()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    tool_memory_stats_with_conn(engine, &conn)
}

pub fn tool_memory_stats_with_conn(
    engine: &HermesEngine,
    conn: &rusqlite::Connection,
) -> Result<String> {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    let acct = Accountant::from_conn(conn, engine.project_id(), engine.session_id());
    let since_midnight = Duration::from_secs(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            % 86400,
    );

    let session = acct.get_memory_stats(Some(since_midnight))?;
    let cumulative = acct.get_memory_stats(None)?;

    Ok(serde_json::to_string_pretty(&json!({
        "session": {
            "sessions_saved":           session.sessions_saved,
            "searches_with_memory":     session.searches_with_memory_hits,
            "total_memory_hits":        session.total_memory_hits,
            "memory_hit_rate_pct":      format!("{:.1}%", session.memory_hit_rate_pct),
            "recall_hits":              session.recall_hits,
            "recall_tokens_saved":      session.recall_avoidance_tokens_saved,
        },
        "cumulative": {
            "sessions_saved":           cumulative.sessions_saved,
            "searches_with_memory":     cumulative.searches_with_memory_hits,
            "total_memory_hits":        cumulative.total_memory_hits,
            "memory_hit_rate_pct":      format!("{:.1}%", cumulative.memory_hit_rate_pct),
            "recall_hits":              cumulative.recall_hits,
            "recall_tokens_saved":      cumulative.recall_avoidance_tokens_saved,
        }
    }))?)
}

pub fn tool_battery_check(engine: &HermesEngine, args: &serde_json::Value) -> Result<String> {
    let threshold = args["session_threshold"].as_u64().unwrap_or(5);
    let days = args["day_threshold"].as_u64().unwrap_or(7);

    let acct = Accountant::new(
        engine.db().clone(),
        engine.project_id(),
        engine.session_id(),
    );
    let due = acct.battery_review_due(threshold, days)?;

    Ok(serde_json::to_string_pretty(&json!({
        "review_due": due,
        "reason": if due { "Threshold exceeded" } else { "Activity levels healthy" }
    }))?)
}

pub fn write_session_checkpoint(engine: &HermesEngine, root: &Path, ingest: bool) -> Result<()> {
    let date = Utc::now().format("%Y-%m-%d").to_string();
    let time = Utc::now().format("%H:%M:%S").to_string();
    let project = engine.project_id();
    let session = engine.session_id();

    let checkpoints_dir = root.join("memory").join("checkpoints");
    std::fs::create_dir_all(&checkpoints_dir)?;

    let filename = format!("{date}_{session}_checkpoint.json");
    let path = checkpoints_dir.join(filename);

    let checkpoint = json!({
        "date": date,
        "time": time,
        "project_id": project,
        "session_id": session,
        "pid": std::process::id(),
    });

    let content = serde_json::to_string_pretty(&checkpoint)?;
    std::fs::write(&path, &content)?;

    if ingest {
        let _ = ingest_single_file(engine, &path);
    }
    Ok(())
}
