// tools/hermes-engine/src/quality/review.rs
// TRACK-049: LLM-driven code review orchestration — file enumeration, prompt
// construction, evidence validation, secret scrubbing, and finding merging.

use anyhow::{Context, Result};
use serde::Deserialize;
use shared_rust::llm_gateway_client::{LlmGatewayClient, Message};
use std::fs;
use std::path::{Path, PathBuf};

use crate::quality::state::Finding;
use crate::quality::zone::{classify_zone, Zone};

const CALLER_ID: &str = "hermes-quality";

// ---------------------------------------------------------------------------
// Quality dimensions (QD-01 … QD-14) — mapped from copilot-instructions.md
// ---------------------------------------------------------------------------

pub struct Dimension {
    pub id: &'static str,
    pub tier: &'static str,
    pub name: &'static str,
    pub description: &'static str,
}

pub static DIMENSIONS: [Dimension; 16] = [
    Dimension { id: "QD-01", tier: "T4", name: "Handler thinness",
        description: "Handlers must not contain business logic or direct DB calls. A handler only parses the request, calls a service function, and maps the result to HTTP." },
    Dimension { id: "QD-02", tier: "T4", name: "Layer isolation",
        description: "DB calls must live in store_*/cosmos_* modules only. Services call store functions. Handlers never touch the DB layer. Check import direction." },
    Dimension { id: "QD-03", tier: "T3", name: "React component purity",
        description: "React components must not contain API calls (fetch/axios/useEffect with fetch). Data fetching belongs in hooks/ or services/api.ts." },
    Dimension { id: "QD-04", tier: "T3", name: "Error handling",
        description: "No unwrap() or expect() in production code paths outside tests. Use ? operator or match/map_err with meaningful error propagation." },
    Dimension { id: "QD-05", tier: "T3", name: "Type safety",
        description: "No TypeScript 'any' without an accompanying '// SAFETY:' comment explaining why no typed alternative exists." },
    Dimension { id: "QD-06", tier: "T4", name: "Secret hygiene",
        description: "No hardcoded credentials, API keys, tokens, or connection strings. All secrets must come from environment variables or Azure Key Vault." },
    Dimension { id: "QD-07", tier: "T2", name: "Naming conventions",
        description: "Rust: snake_case functions, PascalCase types/structs. React: PascalCase components, camelCase hooks with 'use' prefix. TS interfaces: PascalCase." },
    Dimension { id: "QD-08", tier: "T2", name: "File/method size",
        description: "Source files must not exceed 300 lines. Functions/methods must not exceed 50 lines. Refactor if either limit is breached." },
    Dimension { id: "QD-09", tier: "T3", name: "Concurrency correctness",
        description: "Shared async state must use Arc<T> not Rc<T>. Mutable shared state must use Arc<RwLock<T>>. No raw mutexes held across .await points." },
    Dimension { id: "QD-10", tier: "T4", name: "Shared state isolation",
        description: "No cross-service shared mutable state. Services communicate via HTTP APIs only. Check for direct cross-crate struct references." },
    Dimension { id: "QD-11", tier: "T4", name: "Query parameterization",
        description: "Cosmos DB and SQL queries must use @param parameter syntax. Never interpolate user input into query strings." },
    Dimension { id: "QD-12", tier: "T2", name: "Abstraction coherence",
        description: "Each module has a single responsibility. No collapsed layers where handler+service+store logic lives in one function or file." },
    Dimension { id: "QD-13", tier: "T1", name: "Convention consistency",
        description: "Same error format, logging approach, and patterns used throughout. Check for inconsistent error types, mixed logging (eprintln vs tracing), etc." },
    Dimension { id: "QD-14", tier: "T2", name: "TDD coverage signal",
        description: "Non-trivial business logic (functions > 10 lines) must have at least one corresponding test referencing the function name." },
    Dimension { id: "QD-15", tier: "T2", name: "Code conciseness",
        description: "Code should stay concise and efficient: flag needless temporary collections, repeated passes, duplicated branching, or abstraction layers that obscure a straightforward implementation." },
    Dimension { id: "QD-16", tier: "T3", name: "Algorithmic efficiency",
        description: "Review dominant algorithmic complexity and repeated scans. If you find a material complexity concern, estimate the Big-O and judge whether it is likely avoidable or likely unavoidable for this context." },
];

pub fn all_dimensions() -> &'static [Dimension] {
    &DIMENSIONS
}

// ---------------------------------------------------------------------------
// Internal LLM response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Default)]
struct LlmFinding {
    tier: Option<String>,
    line_hint: Option<u32>,
    description: Option<String>,
    evidence: Option<String>,
}

// ---------------------------------------------------------------------------
// File enumeration — returns (path, zone) for reviewable files
// ---------------------------------------------------------------------------

const REVIEWABLE_EXTS: [&str; 9] = ["rs", "ts", "tsx", "js", "jsx", "py", "ps1", "sh", "bat"];
/// ~200 lines × 100 chars — kept low to respect LLM context budget.
const MAX_FILE_CHARS: usize = 20_000;

pub fn enumerate_files(root: &Path) -> Vec<(PathBuf, Zone)> {
    let mut out = Vec::new();
    if root.is_file() {
        push_reviewable_file(root, &mut out, true);
        return out;
    }
    collect_files(root, &mut out);
    out
}

fn collect_files(dir: &Path, out: &mut Vec<(PathBuf, Zone)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if !classify_zone(&path).is_excluded() {
                collect_files(&path, out);
            }
        } else {
            push_reviewable_file(&path, out, false);
        }
    }
}

fn push_reviewable_file(path: &Path, out: &mut Vec<(PathBuf, Zone)>, include_excluded: bool) {
    if !is_reviewable_ext(path) {
        return;
    }
    let zone = classify_zone(path);
    if include_excluded || !zone.is_excluded() {
        out.push((path.to_path_buf(), zone));
    }
}

fn is_reviewable_ext(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    REVIEWABLE_EXTS.contains(&ext)
}

// ---------------------------------------------------------------------------
// Review entry point
// ---------------------------------------------------------------------------

/// Review a single file against one dimension, returning validated findings.
pub fn review_file_dimension(
    client: &LlmGatewayClient,
    file_path: &Path,
    content: &str,
    zone: &Zone,
    dim: &Dimension,
) -> Result<Vec<Finding>> {
    let snippet: String = content.chars().take(MAX_FILE_CHARS).collect();
    let prompt = build_prompt(file_path, &snippet, zone, dim);
    let raw = call_llm(client, &prompt)?;
    Ok(raw
        .into_iter()
        .filter_map(|rf| validate_and_build_finding(rf, content, file_path, zone, dim))
        .collect())
}

fn validate_and_build_finding(
    rf: LlmFinding,
    content: &str,
    file_path: &Path,
    zone: &Zone,
    dim: &Dimension,
) -> Option<Finding> {
    let evidence = rf.evidence.as_deref().unwrap_or("").trim().to_string();
    let description = rf.description.as_deref().unwrap_or("").trim().to_string();
    if evidence.len() < 8 || description.is_empty() {
        return None; // Evidence too short or missing description
    }
    if !content.contains(&evidence) {
        return None; // Evidence must be verbatim in file content
    }
    if contains_secret_pattern(&evidence) {
        return None; // Secret scrub: never persist secret-looking evidence
    }
    let tier = rf.tier.as_deref().unwrap_or(dim.tier);
    let file_str = file_path.to_string_lossy().replace('\\', "/");
    Some(Finding::new(
        dim.id,
        tier,
        zone.as_str(),
        file_str,
        rf.line_hint,
        description,
        evidence,
    ))
}

// ---------------------------------------------------------------------------
// Prompt construction
// ---------------------------------------------------------------------------

fn build_prompt(file_path: &Path, content: &str, zone: &Zone, dim: &Dimension) -> String {
    let extra = match dim.id {
        "QD-15" => "Focus on material opportunities to make the code shorter or simpler without changing behavior. Ignore naming or formatting-only issues.",
        "QD-16" => "For every finding, include `Complexity: O(...)`. Then include `Verdict: likely avoidable` or `Verdict: likely unavoidable`. Return [] if the dominant complexity looks acceptable for the problem.",
        _ => "",
    };
    format!(
        "You are a senior Rust/React architect reviewing code for a strict architecture project.\n\
         File zone: {zone}\nFile: {path}\nCode:\n```\n{content}\n```\n\n\
         Review ONLY for dimension {dim_id} ({dim_name}): {dim_desc}\n\
         Extra review rules: {extra}\n\n\
         Return a JSON array of findings (or [] if none):\n\
         [{{\"tier\": \"{tier}\", \"line_hint\": <int|null>, \
         \"description\": \"<one-sentence violation>\", \
         \"evidence\": \"<verbatim substring from code above>\"}}]\n\
         IMPORTANT: evidence MUST be an exact substring copied from the code. \
         Do not paraphrase. Return [] if no violation.",
        zone = zone.as_str(),
        path = file_path.to_string_lossy(),
        content = content,
        dim_id = dim.id,
        dim_name = dim.name,
        dim_desc = dim.description,
        extra = extra,
        tier = dim.tier,
    )
}

// ---------------------------------------------------------------------------
// LLM call via unified LlmGatewayClient
// ---------------------------------------------------------------------------

fn call_llm(client: &LlmGatewayClient, prompt: &str) -> Result<Vec<LlmFinding>> {
    let system = Message::system("You are a code reviewer. Output ONLY a raw JSON array — no prose, no markdown, no explanations. Just the JSON array.");
    let user = Message::user(prompt);
    let completion = client
        .blocking_chat(None, &[system, user], CALLER_ID)
        .context("LLM quality review call failed")?;
    let content = extract_json_array(&completion.text);
    Ok(serde_json::from_str(&content).unwrap_or_default())
}

fn extract_json_array(content: &str) -> String {
    // Strip Qwen3 <think>...</think> reasoning block if present; answer follows.
    let stripped = if let (Some(ts), Some(te)) = (content.find("<think>"), content.find("</think>"))
    {
        if te > ts {
            &content[te + "</think>".len()..]
        } else {
            content
        }
    } else {
        content
    };
    let t = stripped.trim();
    if let (Some(s), Some(e)) = (t.find('['), t.rfind(']')) {
        if e > s {
            return t[s..=e].to_string();
        }
    }
    "[]".to_string()
}

// ---------------------------------------------------------------------------
// Secret scrub — reject evidence containing secret-looking patterns
// ---------------------------------------------------------------------------

const SECRET_PATTERNS: [&str; 7] = [
    "bearer ",
    "api_key",
    "apikey",
    "password=",
    "secret=",
    "master_key",
    "private_key",
];

fn contains_secret_pattern(text: &str) -> bool {
    let lower = text.to_lowercase();
    SECRET_PATTERNS.iter().any(|p| lower.contains(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enumerate_files_accepts_explicit_tooling_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("tools").join("demo.ps1");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, "Write-Host 'ok'").unwrap();

        let files = enumerate_files(&file);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, file);
        assert_eq!(files[0].1, Zone::Tooling);
    }

    #[test]
    fn build_prompt_requires_complexity_verdict_for_qd16() {
        let prompt = build_prompt(
            Path::new("src/demo.rs"),
            "fn demo() {}",
            &Zone::Production,
            &DIMENSIONS[15],
        );

        assert!(prompt.contains("Complexity: O(...)"));
        assert!(prompt.contains("Verdict: likely avoidable"));
        assert!(prompt.contains("Verdict: likely unavoidable"));
    }
}
