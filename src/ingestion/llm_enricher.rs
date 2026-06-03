// tools/hermes-engine/src/ingestion/llm_enricher.rs
//
// Optional LLM enrichment pass, inspired by google/langextract:
// calls llm-gateway-rust to produce a structured summary for each ingested code chunk.
//
// Protocol boundary: this module communicates with llm-gateway over HTTP.
// Any network/parse failure is handled gracefully — enrichment silently falls back
// to the existing regex-derived summary so offline indexing always works.

use serde::Deserialize;
use shared_rust::llm_gateway_client::{LlmGatewayClient, Message};
use std::sync::{Arc, Condvar, Mutex};
use tracing::warn;

const DEFAULT_GATEWAY_URL: &str = "http://localhost:3001";
const CALLER_ID: &str = "hermes-enricher";
/// Max chars of chunk content to send; keeps prompt tokens low.
const CONTENT_PREVIEW_CHARS: usize = 800;
/// Max LLM output tokens — a structured JSON summary needs ~60-100 tokens.
const DEFAULT_MAX_IN_FLIGHT: usize = 4;
const MAX_PURPOSE_WORDS: usize = 15;
const ALLOWED_LAYERS: [&str; 9] = [
    "handler",
    "service",
    "store",
    "component",
    "hook",
    "type",
    "config",
    "test",
    "other",
];

// ---------------------------------------------------------------------------
// Public surface
// ---------------------------------------------------------------------------

pub struct LlmEnricher {
    client: LlmGatewayClient,
    max_in_flight: usize,
    in_flight: Arc<(Mutex<usize>, Condvar)>,
}

impl LlmEnricher {
    /// Build an enricher from `HERMES_LLM_GATEWAY_URL` env var (falls back to localhost:3001).
    pub fn from_env() -> Self {
        let base_url = std::env::var("LLM_GATEWAY_URL")
            .or_else(|_| std::env::var("HERMES_LLM_GATEWAY_URL"))
            .unwrap_or_else(|_| DEFAULT_GATEWAY_URL.to_string());
        let api_key = std::env::var("LLM_GATEWAY_API_KEY")
            .or_else(|_| std::env::var("HERMES_LLM_GATEWAY_KEY"))
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        let client = LlmGatewayClient::new(reqwest::Client::new(), base_url, api_key);
        let max_in_flight = std::env::var("HERMES_LLM_ENRICH_MAX_IN_FLIGHT")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(DEFAULT_MAX_IN_FLIGHT);
        Self::new(client, max_in_flight)
    }

    pub fn new(client: LlmGatewayClient, max_in_flight: usize) -> Self {
        Self {
            client,
            max_in_flight,
            in_flight: Arc::new((Mutex::new(0), Condvar::new())),
        }
    }

    pub fn enrich(&self, file_path: &str, chunk_name: &str, content: &str) -> Option<String> {
        let _in_flight_guard = self.acquire_slot()?;
        let snippet: String = content.chars().take(CONTENT_PREVIEW_CHARS).collect();

        let system = Message::system("You are a code analysis assistant. Respond ONLY with valid JSON — no markdown fences, no extra text.");
        let user_content = format!(
            "Analyze this code chunk and respond with EXACTLY this JSON (no other text):\n\
            {{\"layer\": \"<handler|service|store|component|hook|type|config|test|other>\", \
            \"purpose\": \"<one sentence, max 15 words>\", \
            \"deps\": [\"SymbolA\", \"SymbolB\"]}}\n\n\
            File: {file_path}\nChunk: {chunk_name}\n\n```\n{snippet}\n```"
        );
        let user = Message::user(&user_content);

        match self.client.blocking_chat(None, &[system, user], CALLER_ID) {
            Ok(completion) => parse_enrichment(&completion.text),
            Err(e) => {
                warn!(error = %e, "LLM enrichment failed — using regex summary");
                None
            }
        }
    }

    fn acquire_slot(&self) -> Option<InFlightGuard> {
        let (lock, condvar) = &*self.in_flight;
        let mut count = lock.lock().ok()?;
        while *count >= self.max_in_flight {
            count = condvar.wait(count).ok()?;
        }
        *count += 1;
        Some(InFlightGuard {
            in_flight: Arc::clone(&self.in_flight),
        })
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct EnrichResult {
    layer: String,
    purpose: String,
    deps: Vec<String>,
}

struct InFlightGuard {
    in_flight: Arc<(Mutex<usize>, Condvar)>,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        let (lock, condvar) = &*self.in_flight;
        if let Ok(mut count) = lock.lock() {
            if *count > 0 {
                *count -= 1;
            }
            condvar.notify_one();
        }
    }
}

/// Parse raw LLM text into a formatted summary string.
/// Strips markdown fences, deserialises JSON, formats result.
pub fn parse_enrichment(raw: &str) -> Option<String> {
    let cleaned = raw
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    let result: EnrichResult = match serde_json::from_str(cleaned) {
        Ok(r) => r,
        Err(e) => {
            warn!(error = %e, raw = %cleaned, "Could not parse enrichment JSON — using regex summary");
            return None;
        }
    };

    let layer = result.layer.trim().to_lowercase();
    if !ALLOWED_LAYERS.contains(&layer.as_str()) {
        warn!(layer = %result.layer, "Invalid enrichment layer — using regex summary");
        return None;
    }

    let mut purpose = normalize_purpose(&result.purpose)?;
    let deps: Vec<String> = result
        .deps
        .into_iter()
        .map(|dep| dep.trim().to_string())
        .filter(|dep| !dep.is_empty())
        .collect();

    let deps_str = if deps.is_empty() {
        String::new()
    } else {
        format!(" Uses: {}", deps.join(", "))
    };

    if !purpose.ends_with('.') && !purpose.ends_with('!') && !purpose.ends_with('?') {
        purpose.push('.');
    }
    Some(format!("[{}] {}{}", layer, purpose, deps_str))
}

fn normalize_purpose(raw: &str) -> Option<String> {
    let purpose = raw.trim();
    if purpose.is_empty() {
        warn!("Empty enrichment purpose — using regex summary");
        return None;
    }
    if purpose.split_whitespace().count() > MAX_PURPOSE_WORDS {
        warn!(
            words = purpose.split_whitespace().count(),
            max_words = MAX_PURPOSE_WORDS,
            "Purpose exceeds word limit — using regex summary"
        );
        return None;
    }
    Some(purpose.trim_end_matches('.').to_string())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_llm_enricher_enrich_returns_none_when_gateway_unreachable() {
        let client = LlmGatewayClient::new(
            reqwest::Client::new(),
            "http://localhost:19999".to_string(),
            None,
        );
        let enricher = LlmEnricher::new(client, 1);
        let result = enricher.enrich("src/foo.rs", "bar_fn", "fn bar() {}");
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_enrichment_valid_json_returns_formatted_string() {
        let raw = r#"{"layer":"service","purpose":"Coordinates token budget enforcement across callers.","deps":["BudgetStore","CallerConfig"]}"#;
        let result = parse_enrichment(raw).unwrap();
        assert_eq!(
            result,
            "[service] Coordinates token budget enforcement across callers. Uses: BudgetStore, CallerConfig"
        );
    }

    #[test]
    fn test_parse_enrichment_no_deps_omits_uses_clause() {
        let raw = r#"{"layer":"type","purpose":"Defines the ChatRequest DTO.","deps":[]}"#;
        let result = parse_enrichment(raw).unwrap();
        assert_eq!(result, "[type] Defines the ChatRequest DTO.");
    }

    #[test]
    fn test_parse_enrichment_markdown_fence_stripped() {
        let raw =
            "```json\n{\"layer\":\"handler\",\"purpose\":\"Routes HTTP requests to services.\",\"deps\":[]}\n```";
        let result = parse_enrichment(raw).unwrap();
        assert_eq!(result, "[handler] Routes HTTP requests to services.");
    }

    #[test]
    fn test_parse_enrichment_invalid_json_returns_none() {
        assert!(parse_enrichment("not json at all").is_none());
    }

    #[test]
    fn test_llm_enricher_from_env_uses_default_when_no_var() {
        // Safety: remove the env var for this test; other tests are not using it.
        unsafe { std::env::remove_var("HERMES_LLM_GATEWAY_URL") };
        let enricher = LlmEnricher::from_env();
        assert_eq!(enricher.client.base_url(), DEFAULT_GATEWAY_URL);
    }

    #[test]
    fn test_parse_enrichment_invalid_layer_returns_none() {
        let raw = r#"{"layer":"controller","purpose":"Routes requests.","deps":[]}"#;
        assert!(parse_enrichment(raw).is_none());
    }

    #[test]
    fn test_parse_enrichment_too_many_words_returns_none() {
        let raw = r#"{"layer":"service","purpose":"One two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen.","deps":[]}"#;
        assert!(parse_enrichment(raw).is_none());
    }
}
