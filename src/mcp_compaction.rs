use anyhow::Result;
use serde_json::{json, Value};
use std::path::Path;

use crate::graph::KnowledgeGraph;
use crate::ingestion::IngestionPipeline;
use crate::mcp_compaction_support::{
    array_field, build_handover, dedupe, default_continuation_prompt, first_non_empty,
    first_non_empty_array, next_handover_path, preview, string_field, summary_word_budget,
    truncate_words,
};
use crate::HermesEngine;

const DEFAULT_TARGET_TOKEN_BUDGET: usize = 1200;

pub fn tool_compact_session(engine: &HermesEngine, root: &Path, args: &Value) -> Result<String> {
    let request = CompactRequest::from_args(args)?;
    let artifact = build_artifact(&request);
    let mut handover_path = None;
    let mut indexed = false;

    if request.persist_handover {
        let relative_path = next_handover_path(root, &request.topic);
        let absolute_path = root.join(&relative_path);
        if let Some(parent) = absolute_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&absolute_path, &artifact.handover)?;
        let ingest_result = ingest_single_file(engine, &absolute_path);
        indexed = ingest_result.is_ok();
        if let Err(ref err) = ingest_result {
            eprintln!("[hermes] immediate ingest after compact_session failed: {err}");
        }
        handover_path = Some(relative_path.to_string_lossy().replace('\\', "/"));
    }

    serde_json::to_string_pretty(&json!({
        "summary": artifact.summary,
        "handover": artifact.handover,
        "relevant_files": artifact.relevant_files,
        "next_actions": artifact.next_actions,
        "target_token_budget": request.target_token_budget,
        "persisted": request.persist_handover,
        "handover_path": handover_path,
        "indexed": indexed,
    }))
    .map_err(Into::into)
}

#[derive(Default)]
struct CompactRequest {
    topic: String,
    summary: String,
    recent_messages: Vec<String>,
    accomplished: Vec<String>,
    files_touched: Vec<String>,
    decisions: Vec<String>,
    problems: Vec<String>,
    actions: Vec<String>,
    active_constraints: Vec<String>,
    recent_errors: Vec<String>,
    continuation_prompt: String,
    target_token_budget: usize,
    persist_handover: bool,
}

struct CompactArtifact {
    summary: String,
    handover: String,
    relevant_files: Vec<String>,
    next_actions: Vec<String>,
}

impl CompactRequest {
    fn from_args(args: &Value) -> Result<Self> {
        let topic = first_non_empty(args, &["topic", "task"]);
        anyhow::ensure!(
            !topic.is_empty(),
            "hermes_compact_session requires 'topic' or 'task'"
        );

        Ok(Self {
            topic,
            summary: string_field(args, "summary"),
            recent_messages: array_field(args, "recent_messages"),
            accomplished: first_non_empty_array(args, &["accomplished", "summarized_steps"]),
            files_touched: array_field(args, "files_touched"),
            decisions: array_field(args, "decisions"),
            problems: array_field(args, "problems"),
            actions: array_field(args, "actions"),
            active_constraints: array_field(args, "active_constraints"),
            recent_errors: array_field(args, "recent_errors"),
            continuation_prompt: string_field(args, "continuation_prompt"),
            target_token_budget: args
                .get("target_token_budget")
                .and_then(|value| value.as_u64())
                .map(|value| value as usize)
                .filter(|value| *value > 0)
                .unwrap_or(DEFAULT_TARGET_TOKEN_BUDGET),
            persist_handover: args
                .get("persist_handover")
                .and_then(|value| value.as_bool())
                .unwrap_or(false),
        })
    }
}

fn build_artifact(request: &CompactRequest) -> CompactArtifact {
    let relevant_files = dedupe(&request.files_touched);
    let completed_steps = if request.accomplished.is_empty() {
        request.recent_messages.clone()
    } else {
        request.accomplished.clone()
    };
    let next_actions = if request.actions.is_empty() {
        request.problems.clone()
    } else {
        request.actions.clone()
    };
    let continuation_prompt = if request.continuation_prompt.is_empty() {
        default_continuation_prompt(&request.topic, &next_actions, &relevant_files)
    } else {
        request.continuation_prompt.clone()
    };

    CompactArtifact {
        summary: build_summary(request, &relevant_files, &next_actions),
        handover: build_handover(
            &request.topic,
            &request.summary,
            &request.decisions,
            &request.active_constraints,
            &request.recent_errors,
            &request.problems,
            &completed_steps,
            &relevant_files,
            &next_actions,
            &continuation_prompt,
        ),
        relevant_files,
        next_actions,
    }
}

fn build_summary(
    request: &CompactRequest,
    relevant_files: &[String],
    next_actions: &[String],
) -> String {
    let mut parts = vec![format!("Task: {}.", request.topic)];
    if !request.summary.is_empty() {
        parts.push(request.summary.clone());
    }
    if !request.accomplished.is_empty() {
        parts.push(format!(
            "Completed: {}.",
            preview(&request.accomplished, 3, "nothing recorded")
        ));
    } else if !request.recent_messages.is_empty() {
        parts.push(format!(
            "Recent context: {}.",
            preview(&request.recent_messages, 2, "none")
        ));
    }
    if !request.decisions.is_empty() {
        parts.push(format!(
            "Decisions: {}.",
            preview(&request.decisions, 3, "none")
        ));
    }
    if !request.problems.is_empty() {
        parts.push(format!(
            "Problems: {}.",
            preview(&request.problems, 2, "none")
        ));
    }
    if !next_actions.is_empty() {
        parts.push(format!(
            "Next actions: {}.",
            preview(next_actions, 3, "review the handover")
        ));
    }
    if !relevant_files.is_empty() {
        parts.push(format!(
            "Relevant files: {}.",
            preview(relevant_files, 4, "none")
        ));
    }

    truncate_words(
        parts.join(" "),
        summary_word_budget(request.target_token_budget),
    )
}

fn ingest_single_file(engine: &HermesEngine, path: &Path) -> Result<()> {
    let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
    let pipeline = IngestionPipeline::new(&graph);
    let env_acc = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    pipeline.ingest_file(path, &env_acc)?;
    engine.invalidate_search_cache();
    Ok(())
}
