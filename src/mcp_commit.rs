use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};

#[derive(Debug, Clone, Default)]
pub struct CommitMessageInput {
    pub task_model: Option<String>,
    pub decision_doc: Option<String>,
    pub session_note: Option<String>,
    pub docs: Vec<String>,
    pub pipelines: Vec<u32>,
    pub changes: Vec<String>,
    pub body: Option<String>,
}

pub fn input_from_cli_args(args: &[String]) -> Result<(String, CommitMessageInput)> {
    let subject = args.get(2).map(String::as_str).unwrap_or("").trim();
    if subject.is_empty() {
        bail!("usage: hermes prepare-commit-message <subject> [--task <id>] [--decision <path>] [--session <path>] [--docs <csv>] [--pipeline <csv>] [--changes <csv>] [--body <text>]");
    }

    let mut input = CommitMessageInput::default();
    let mut i = 3usize;
    while i < args.len() {
        match args[i].as_str() {
            "--task" => {
                input.task_model = args.get(i + 1).cloned();
                i += 2;
            }
            "--decision" => {
                input.decision_doc = args.get(i + 1).cloned();
                i += 2;
            }
            "--session" => {
                input.session_note = args.get(i + 1).cloned();
                i += 2;
            }
            "--docs" => {
                if let Some(v) = args.get(i + 1) {
                    input.docs = parse_csv_list(v);
                }
                i += 2;
            }
            "--pipeline" => {
                if let Some(v) = args.get(i + 1) {
                    input.pipelines = parse_pipeline_list(v)?;
                }
                i += 2;
            }
            "--changes" => {
                if let Some(v) = args.get(i + 1) {
                    input.changes = parse_csv_list(v);
                }
                i += 2;
            }
            "--body" => {
                input.body = args.get(i + 1).cloned();
                i += 2;
            }
            _ => {
                i += 1;
            }
        }
    }

    if input.pipelines.is_empty() {
        input.pipelines = infer_pipeline_ids(&input.changes);
    }

    Ok((subject.to_string(), input))
}

pub fn input_from_json(args: &Value) -> Result<(String, CommitMessageInput)> {
    let subject = args["subject"].as_str().unwrap_or("").trim();
    if subject.is_empty() {
        bail!("hermes_prepare_commit_message requires 'subject'");
    }

    let docs = if let Some(arr) = args["docs"].as_array() {
        arr.iter()
            .filter_map(|v| v.as_str().map(ToString::to_string))
            .collect::<Vec<_>>()
    } else {
        parse_csv_list(args["docs"].as_str().unwrap_or(""))
    };

    let changes = if let Some(arr) = args["changes"].as_array() {
        arr.iter()
            .filter_map(|v| v.as_str().map(ToString::to_string))
            .collect::<Vec<_>>()
    } else {
        parse_csv_list(args["changes"].as_str().unwrap_or(""))
    };

    let mut pipelines = if let Some(arr) = args["pipelines"].as_array() {
        arr.iter()
            .filter_map(|v| {
                v.as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .or_else(|| v.as_str().and_then(|s| s.parse::<u32>().ok()))
            })
            .collect::<Vec<_>>()
    } else {
        parse_pipeline_list(args["pipelines"].as_str().unwrap_or(""))?
    };

    if pipelines.is_empty() {
        pipelines = infer_pipeline_ids(&changes);
    }

    let input = CommitMessageInput {
        task_model: args["task_model"].as_str().map(ToString::to_string),
        decision_doc: args["decision_doc"].as_str().map(ToString::to_string),
        session_note: args["session_note"].as_str().map(ToString::to_string),
        docs,
        pipelines,
        changes,
        body: args["body"].as_str().map(ToString::to_string),
    };

    Ok((subject.to_string(), input))
}

pub fn render_commit_message(subject: &str, input: &CommitMessageInput) -> String {
    let mut out = String::new();
    out.push_str(subject.trim());
    out.push_str("\n\n");

    if let Some(body) = input.body.as_deref() {
        out.push_str(body.trim());
        out.push_str("\n\n");
    } else {
        out.push_str("Attach task, decision, docs, and pipeline context so SRE build healing can trace intent quickly.\n\n");
    }

    if let Some(v) = input.task_model.as_deref() {
        out.push_str(&format!("Task-Model: {}\n", v));
    }
    if let Some(v) = input.decision_doc.as_deref() {
        out.push_str(&format!("Decision-Doc: {}\n", v));
    }
    if let Some(v) = input.session_note.as_deref() {
        out.push_str(&format!("Session-Note: {}\n", v));
    }
    if !input.docs.is_empty() {
        out.push_str(&format!("Docs: {}\n", input.docs.join(", ")));
    }
    if !input.pipelines.is_empty() {
        let ids = input
            .pipelines
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>();
        out.push_str(&format!("Pipeline: {}\n", ids.join(",")));
    }

    out
}

pub fn tool_prepare_commit_message(args: &Value) -> Result<String> {
    let (subject, input) = input_from_json(args)?;
    let message = render_commit_message(&subject, &input);
    Ok(serde_json::to_string_pretty(&json!({
        "subject": subject,
        "message": message,
        "inferred_pipelines": input.pipelines,
        "docs": input.docs,
        "changes": input.changes,
    }))?)
}

fn parse_csv_list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn parse_pipeline_list(raw: &str) -> Result<Vec<u32>> {
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }

    let mut out = Vec::new();
    for part in raw.split(',').map(str::trim).filter(|v| !v.is_empty()) {
        let id = part
            .parse::<u32>()
            .map_err(|_| anyhow!("invalid pipeline id: {part}"))?;
        out.push(id);
    }
    Ok(out)
}

pub fn infer_pipeline_ids(changes: &[String]) -> Vec<u32> {
    let mut ids = Vec::new();

    for path in changes {
        let normalized = path.replace('\\', "/").to_lowercase();
        maybe_push(&mut ids, &normalized, "chartapp/chartapp-server-rust/", 9);
        maybe_push(
            &mut ids,
            &normalized,
            "chartapp/mastermind-daemon-rust/",
            10,
        );
        maybe_push(&mut ids, &normalized, "chartapp/chartapp.client/", 13);
        maybe_push(&mut ids, &normalized, "infra/", 14);
        maybe_push(&mut ids, &normalized, "chartapp/trainer-worker-rust/", 8);
        maybe_push(&mut ids, &normalized, "chartapp/telegram-gateway-rust/", 11);
        maybe_push(&mut ids, &normalized, "chartapp/llm-gateway-rust/", 7);
        maybe_push(&mut ids, &normalized, "chartapp/doctor-service-rust/", 12);
        maybe_push(&mut ids, &normalized, "chartapp/watchdog-rust/", 5);
        maybe_push(
            &mut ids,
            &normalized,
            "chartapp/local-agent-training-worker/",
            3,
        );
        maybe_push(&mut ids, &normalized, "chartapp/android-app/", 6);
        maybe_push(&mut ids, &normalized, "chartapp/codex-worker-rust/", 15);
        maybe_push(&mut ids, &normalized, "tools/ccterm/", 18);
    }

    ids
}

fn maybe_push(ids: &mut Vec<u32>, path: &str, prefix: &str, pipeline_id: u32) {
    if path.starts_with(prefix) && !ids.contains(&pipeline_id) {
        ids.push(pipeline_id);
    }
}

pub fn tool_validate_commit_context(message: &str) -> Result<String> {
    let all_trailers = [
        "Task-Model",
        "Decision-Doc",
        "Session-Note",
        "Docs",
        "Pipeline",
    ];

    // First pass: collect what's present
    let present: Vec<String> = all_trailers
        .iter()
        .filter(|t| message.contains(&format!("{t}:")))
        .map(|t| t.to_string())
        .collect();

    let mut errors: Vec<String> = Vec::new();

    // Subject line check
    let first_line = message.lines().next().unwrap_or("");
    if first_line.len() > 72 {
        errors.push("Subject line exceeds 72 characters".to_string());
    }

    // Second pass: determine what's missing, allowing Decision-Doc <-> Session-Note alternative
    let has_decision_or_session = present.iter().any(|t| t == "Decision-Doc" || t == "Session-Note");
    let mut missing: Vec<String> = Vec::new();

    for trailer in &all_trailers {
        if present.contains(&trailer.to_string()) {
            continue;
        }
        match *trailer {
            "Decision-Doc" | "Session-Note" => {
                if !has_decision_or_session {
                    missing.push(trailer.to_string());
                }
            }
            _ => {
                missing.push(trailer.to_string());
            }
        }
    }

    let valid = missing.is_empty() && errors.is_empty();

    let result = serde_json::json!({
        "valid": valid,
        "present": present,
        "missing": missing,
        "errors": errors,
    });

    Ok(serde_json::to_string_pretty(&result)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_valid_message() {
        let msg = "feat: add widget\n\nTask-Model: task://abc\nDecision-Doc: docs/x.md\nDocs: docs/api.md\nPipeline: 18\n";
        let result = tool_validate_commit_context(msg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert!(v["valid"].as_bool().unwrap());
    }

    #[test]
    fn test_validate_missing_trailers() {
        let msg = "feat: add widget\n\nTask-Model: task://abc\n";
        let result = tool_validate_commit_context(msg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert!(!v["valid"].as_bool().unwrap());
        let missing: Vec<&str> = v["missing"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|m| m.as_str())
            .collect();
        assert!(missing.contains(&"Docs"));
        assert!(missing.contains(&"Pipeline"));
    }

    #[test]
    fn test_validate_decision_or_session_is_enough() {
        // Decision-Doc without Session-Note should be valid
        let msg = "fix: resolve crash\n\nTask-Model: task://x\nDecision-Doc: docs/crash.md\nDocs: changelog.md\nPipeline: 9\n";
        let result = tool_validate_commit_context(msg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert!(v["valid"].as_bool().unwrap());

        // Session-Note without Decision-Doc should also be valid
        let msg2 = "fix: resolve crash\n\nTask-Model: task://x\nSession-Note: memory/sessions/foo.md\nDocs: changelog.md\nPipeline: 9\n";
        let result2 = tool_validate_commit_context(msg2).unwrap();
        let v2: serde_json::Value = serde_json::from_str(&result2).unwrap();
        assert!(v2["valid"].as_bool().unwrap());
    }

    #[test]
    fn test_validate_requires_both_task_model_and_pipeline() {
        let msg = "chore: update deps\n\nDocs: readme.md\n";
        let result = tool_validate_commit_context(msg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert!(!v["valid"].as_bool().unwrap());
        let missing: Vec<&str> = v["missing"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|m| m.as_str())
            .collect();
        assert!(missing.contains(&"Task-Model"));
        assert!(missing.contains(&"Pipeline"));
    }

    #[test]
    fn test_infer_pipeline_ids_ccterm() {
        let ids = infer_pipeline_ids(&["tools/ccterm/src/web_ui/app.js".to_string()]);
        assert_eq!(ids, vec![18]);
    }

    #[test]
    fn test_render_commit_message_with_trailers() {
        let input = CommitMessageInput {
            task_model: Some("task://abc".to_string()),
            decision_doc: Some("memory/decisions/x.md".to_string()),
            session_note: None,
            docs: vec!["docs/sre-dashboard.md".to_string()],
            pipelines: vec![18],
            changes: vec![],
            body: None,
        };

        let text = render_commit_message("fix(ccterm): x", &input);
        assert!(text.contains("Task-Model: task://abc"));
        assert!(text.contains("Decision-Doc: memory/decisions/x.md"));
        assert!(text.contains("Docs: docs/sre-dashboard.md"));
        assert!(text.contains("Pipeline: 18"));
    }
}
