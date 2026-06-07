// tools/hermes-engine/src/mcp_memory/utils.rs
use crate::graph::KnowledgeGraph;
use crate::ingestion::IngestionPipeline;
use crate::HermesEngine;
use anyhow::Result;
use std::path::Path;

pub(crate) fn ingest_single_file(engine: &HermesEngine, path: &Path) -> Result<()> {
    let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
    let pipeline = IngestionPipeline::new(&graph);
    let env_acc = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    pipeline.ingest_file(path, &env_acc)?;
    engine.invalidate_search_cache();
    Ok(())
}

pub(crate) fn str_array(val: &serde_json::Value, key: &str) -> Vec<String> {
    val[key]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn slugify(text: &str) -> String {
    let slug: String = text
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    let trimmed = slug.trim_matches('-');
    if trimmed.len() > 60 {
        trimmed[..60].trim_end_matches('-').to_string()
    } else {
        trimmed.to_string()
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_md(
    date: &str,
    topic: &str,
    tags: &[String],
    files: &[String],
    summary: &str,
    decisions: &[String],
    problems: &[String],
    actions: &[String],
    metrics: &[(String, u64)],
) -> String {
    let mut md = format!("# Session: {date} \u{2014} {topic}\n\n## Metadata\n- date: {date}\n");
    if !tags.is_empty() {
        md.push_str(&format!("- tags: {}\n", tags.join(", ")));
    }
    if !files.is_empty() {
        md.push_str(&format!("- files-touched: {}\n", files.join(", ")));
    }
    if !metrics.is_empty() {
        md.push_str("- metrics:\n");
        for (k, v) in metrics {
            md.push_str(&format!("  - {k}: {v}\n"));
        }
    }
    md.push('\n');
    md.push_str(&format!("## Summary\n{summary}\n\n"));
    if !decisions.is_empty() {
        md.push_str("## Key Decisions\n");
        for d in decisions {
            md.push_str(&format!("- {d}\n"));
        }
        md.push('\n');
    }
    if !problems.is_empty() {
        md.push_str("## Problems Encountered\n");
        for p in problems {
            md.push_str(&format!("- {p}\n"));
        }
        md.push('\n');
    }
    if !actions.is_empty() {
        md.push_str("## Action Items\n");
        for a in actions {
            md.push_str(&format!("- [ ] {a}\n"));
        }
        md.push('\n');
    }
    md
}
