use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pointer {
    pub id: String,
    pub source: String,
    pub chunk: String,
    pub lines: String,
    pub relevance: f64,
    pub summary: String,
    pub node_type: String,
    pub last_modified: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PointerResponse {
    pub pointers: Vec<Pointer>,
    pub accounting: AccountingReport,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountingReport {
    pub pointer_tokens: u64,
    pub fetched_tokens: u64,
    pub total_tokens: u64,
    pub traditional_rag_estimate: u64,
    pub savings_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchResponse {
    pub pointer_id: String,
    pub content: String,
    pub file_path: String,
    pub start_line: i64,
    pub end_line: i64,
    pub token_count: u64,
}

impl Pointer {
    pub fn estimate_token_count(&self) -> u64 {
        let text = format!(
            "{} {} {} {}",
            self.source, self.chunk, self.lines, self.summary
        );
        let word_count = text.split_whitespace().count() as u64;
        (word_count * 4).div_ceil(3) + 2
    }
}

impl PointerResponse {
    pub fn build(pointers: Vec<Pointer>, fetched_tokens: u64) -> Self {
        let pointer_tokens: u64 = pointers.iter().map(|p| p.estimate_token_count()).sum();
        let traditional_estimate = pointer_tokens * 15;
        let total = pointer_tokens + fetched_tokens;
        let savings_pct = if traditional_estimate > 0 {
            (1.0 - (total as f64 / traditional_estimate as f64)) * 100.0
        } else {
            0.0
        };

        Self {
            pointers,
            accounting: AccountingReport {
                pointer_tokens,
                fetched_tokens,
                total_tokens: total,
                traditional_rag_estimate: traditional_estimate,
                savings_pct: savings_pct.max(0.0),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_token_estimation() {
        let ptr = Pointer {
            id: "abc".to_string(),
            source: "src/main.rs".to_string(),
            chunk: "fn main".to_string(),
            lines: "1-20".to_string(),
            relevance: 0.95,
            summary: "Application entry point".to_string(),
            node_type: "function".to_string(),
            last_modified: None,
        };
        let tokens = ptr.estimate_token_count();
        assert!(tokens > 0 && tokens < 100);
    }

    #[test]
    fn pointer_response_calculates_savings() {
        let ptrs = vec![Pointer {
            id: "1".to_string(),
            source: "src/lib.rs".to_string(),
            chunk: "struct Engine".to_string(),
            lines: "10-30".to_string(),
            relevance: 0.9,
            summary: "Main engine struct with configuration".to_string(),
            node_type: "struct".to_string(),
            last_modified: None,
        }];
        let resp = PointerResponse::build(ptrs, 0);
        assert!(resp.accounting.savings_pct > 0.0);
        assert!(resp.accounting.traditional_rag_estimate > resp.accounting.pointer_tokens);
    }
}
