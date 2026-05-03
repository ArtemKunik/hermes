use chrono::Utc;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub fn build_handover(
    topic: &str,
    summary: &str,
    decisions: &[String],
    active_constraints: &[String],
    recent_errors: &[String],
    problems: &[String],
    completed_steps: &[String],
    relevant_files: &[String],
    next_actions: &[String],
    continuation_prompt: &str,
) -> String {
    let mut handover = format!(
        "# Handover: {}\n\n## Active Task State\n- Current objective: {}\n",
        topic, topic
    );
    if !summary.is_empty() {
        handover.push_str(&format!("- Current status: {}\n", summary));
    }
    handover.push_str("- Completed steps:\n");
    append_numbered(
        &mut handover,
        completed_steps,
        "No completed steps recorded yet.",
    );
    handover.push_str("- Blocked/remaining:\n");
    append_numbered(
        &mut handover,
        next_actions,
        "Review the stored handover and decide the next action.",
    );
    handover.push_str("\n## Critical Context\n");
    handover.push_str(&format!(
        "- Key files: {}\n",
        preview(relevant_files, 6, "none recorded")
    ));
    handover.push_str(&format!(
        "- Decisions: {}\n",
        preview(decisions, 4, "none recorded")
    ));
    handover.push_str(&format!(
        "- Active constraints: {}\n",
        preview(active_constraints, 4, "none recorded")
    ));
    let errors = if recent_errors.is_empty() {
        problems
    } else {
        recent_errors
    };
    handover.push_str(&format!(
        "- Recent errors: {}\n\n",
        preview(errors, 4, "none recorded")
    ));
    handover.push_str("## Continuation Prompt\n");
    handover.push_str(continuation_prompt);
    handover.push('\n');
    handover
}

pub fn default_continuation_prompt(
    topic: &str,
    next_actions: &[String],
    relevant_files: &[String],
) -> String {
    let action = next_actions.first().cloned().unwrap_or_else(|| {
        "review the handover and choose the highest-value next step".to_string()
    });
    let files = if relevant_files.is_empty() {
        "the current task files".to_string()
    } else {
        preview(relevant_files, 3, "the current task files")
    };
    format!(
        "Resume {topic}. Start with {action}. Re-open {files} before making additional changes."
    )
}

pub fn next_handover_path(root: &Path, topic: &str) -> PathBuf {
    let date = Utc::now().format("%Y-%m-%d").to_string();
    let slug = slugify(topic);
    let base_dir = Path::new("memory").join("handover");
    let mut attempt = 1usize;
    loop {
        let file_name = if attempt == 1 {
            format!("{date}-{slug}.md")
        } else {
            format!("{date}-{slug}-{attempt}.md")
        };
        let relative = base_dir.join(file_name);
        if !root.join(&relative).exists() {
            return relative;
        }
        attempt += 1;
    }
}

pub fn first_non_empty(args: &Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| {
            args.get(*key)
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_default()
}

pub fn first_non_empty_array(args: &Value, keys: &[&str]) -> Vec<String> {
    keys.iter()
        .map(|key| array_field(args, key))
        .find(|items| !items.is_empty())
        .unwrap_or_default()
}

pub fn string_field(args: &Value, key: &str) -> String {
    args.get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_default()
}

pub fn array_field(args: &Value, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(|value| value.as_array())
        .map(|items| items.iter().filter_map(normalize_entry).collect())
        .unwrap_or_default()
}

pub fn dedupe(values: &[String]) -> Vec<String> {
    values
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub fn preview(items: &[String], max_items: usize, fallback: &str) -> String {
    if items.is_empty() {
        return fallback.to_string();
    }

    let mut preview = items
        .iter()
        .take(max_items)
        .cloned()
        .collect::<Vec<_>>()
        .join("; ");
    if items.len() > max_items {
        preview.push_str(&format!("; +{} more", items.len() - max_items));
    }
    preview
}

pub fn summary_word_budget(target_token_budget: usize) -> usize {
    (target_token_budget / 8).clamp(30, 120)
}

pub fn truncate_words(text: String, max_words: usize) -> String {
    let words = text.split_whitespace().collect::<Vec<_>>();
    if words.len() <= max_words {
        return text;
    }
    format!(
        "{} ...",
        words
            .into_iter()
            .take(max_words)
            .collect::<Vec<_>>()
            .join(" ")
    )
}

fn append_numbered(buffer: &mut String, items: &[String], fallback: &str) {
    if items.is_empty() {
        buffer.push_str(&format!("1. {}\n", fallback));
        return;
    }

    for (index, item) in items.iter().enumerate() {
        buffer.push_str(&format!("{}. {}\n", index + 1, item));
    }
}

fn normalize_entry(value: &Value) -> Option<String> {
    let text = value
        .as_str()
        .or_else(|| value.get("content").and_then(|inner| inner.as_str()))?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn slugify(text: &str) -> String {
    let slug = text
        .to_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect::<String>();
    let trimmed = slug.trim_matches('-');
    if trimmed.len() > 60 {
        trimmed[..60].trim_end_matches('-').to_string()
    } else {
        trimmed.to_string()
    }
}
