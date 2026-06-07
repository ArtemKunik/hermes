// tools/hermes-engine/src/kb_handler.rs
use anyhow::Result;
use chrono::Utc;
use serde_json::json;
use std::path::Path;

use crate::accounting::Accountant;
use crate::graph::KnowledgeGraph;
use crate::incident_io::str_array;
use crate::mcp_memory::{ingest_single_file, slugify};
use crate::search::{SearchEngine, SearchMode};
use crate::HermesEngine;

/// Write a standalone KB article.
pub fn tool_write_kb_article(
    engine: &HermesEngine,
    root: &Path,
    args: &serde_json::Value,
) -> Result<String> {
    let sub_product = args["sub_product"].as_str().unwrap_or("unknown");
    let title = args["title"].as_str().unwrap_or("");
    anyhow::ensure!(
        !title.is_empty(),
        "hermes_write_kb_article requires 'title'"
    );

    let problem = args["problem"].as_str().unwrap_or("");
    let solution = args["solution"].as_str().unwrap_or("");
    let tags = str_array(args, "tags");

    let date = Utc::now().format("%Y-%m-%d").to_string();
    let slug = args["slug"]
        .as_str()
        .map(String::from)
        .unwrap_or_else(|| slugify(title));

    let dir = root.join("memory").join("kb").join(sub_product);
    std::fs::create_dir_all(&dir)?;

    let path = dir.join(format!("{slug}.md"));
    let existed = path.exists();

    let mut content = format!(
        "# KB: {title}\n\n\
         ## Metadata\n\
         - date: {date}\n\
         - sub_product: {sub_product}\n\
         - tags: {tags}\n\n\
         ## Problem\n\
         {problem}\n\n\
         ## Solution\n\
         {solution}\n",
        tags = tags.join(", ")
    );

    if let Some(root_cause) = args["root_cause"].as_str() {
        content.push_str(&format!("\n## Root Cause\n{root_cause}\n"));
    }
    if let Some(prevention) = args["prevention"].as_str() {
        content.push_str(&format!("\n## Prevention\n{prevention}\n"));
    }

    std::fs::write(&path, &content)?;

    let acct = Accountant::new(
        engine.db().clone(),
        engine.project_id(),
        engine.session_id(),
    );
    acct.record_memory_event(
        if existed { "kb_updated" } else { "kb_created" },
        Some(title),
        Some(&path.to_string_lossy()),
        None,
    )?;

    if let Err(e) = ingest_single_file(engine, &path) {
        eprintln!("[hermes] ingest after write_kb_article failed: {e}");
    }

    Ok(serde_json::to_string_pretty(&json!({
        "status": if existed { "updated" } else { "created" },
        "path": path.to_string_lossy(),
    }))?)
}

/// Search KB articles. Wraps hermes_search but filters results to memory/kb/ paths.
pub fn tool_search_kb(engine: &HermesEngine, args: &serde_json::Value) -> Result<String> {
    let db = engine.read_db().lock().unwrap_or_else(|e| e.into_inner());
    tool_search_kb_with_conn(engine, &db, args)
}

pub fn tool_search_kb_with_conn(
    engine: &HermesEngine,
    conn: &rusqlite::Connection,
    args: &serde_json::Value,
) -> Result<String> {
    let query = args["query"].as_str().unwrap_or("");
    anyhow::ensure!(!query.is_empty(), "hermes_search_kb requires 'query'");
    let filter_sp = args["sub_product"].as_str();

    let graph = KnowledgeGraph::from_conn(conn, engine.project_id());
    let search = SearchEngine::new(&graph, engine.search_cache());
    let results = search.search(query, 20, &SearchMode::Smart)?;

    let kb_prefix = std::path::Path::new("memory").join("kb");
    let kb_prefix_str = kb_prefix.to_string_lossy().replace('\\', "/");

    let filtered: Vec<serde_json::Value> = results
        .pointers
        .into_iter()
        .filter(|p| {
            let src = p.source.replace('\\', "/");
            if !src.contains(&*kb_prefix_str) {
                return false;
            }
            if let Some(sp) = filter_sp {
                let sp_seg = format!("kb/{sp}/");
                src.contains(&sp_seg)
            } else {
                true
            }
        })
        .map(|p| {
            json!({
                "source": p.source,
                "summary": p.summary,
                "relevance": p.relevance,
            })
        })
        .collect();

    Ok(serde_json::to_string_pretty(&json!({
        "query": query,
        "kb_articles": filtered,
        "count": filtered.len(),
    }))?)
}
