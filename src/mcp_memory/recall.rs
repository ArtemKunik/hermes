// tools/hermes-engine/src/mcp_memory/recall.rs
use crate::accounting::Accountant;
use crate::graph::KnowledgeGraph;
use crate::pointer::Pointer;
use crate::search::SearchEngine;
use crate::HermesEngine;
use anyhow::Result;

const RECALL_AVOIDANCE_MULTIPLIER: u64 = 7;
const SESSION_HIT_AVOIDANCE_ESTIMATE: u64 = 1_500;
const RECALL_MAX_QUERY_WORDS: usize = 10;

pub fn tool_recall(engine: &HermesEngine, query: &str) -> Result<String> {
    let db = engine.read_db().lock().unwrap_or_else(|e| e.into_inner());
    tool_recall_with_conn(engine, &db, query)
}

pub fn tool_recall_with_conn(
    engine: &HermesEngine,
    conn: &rusqlite::Connection,
    query: &str,
) -> Result<String> {
    anyhow::ensure!(!query.is_empty(), "hermes_recall requires 'query'");

    let graph = KnowledgeGraph::from_conn(conn, engine.project_id());
    let search = SearchEngine::new(&graph, engine.search_cache());
    let memory_pointers = recall_memory_search(engine, &graph, query, 20)?;
    let related_code = recall_related_code_search(engine, &graph, query, 5)?;

    let mut decision_briefs: Vec<serde_json::Value> = Vec::new();
    let mut fetched_tokens: u64 = 0;

    let mut decision_pointers = Vec::new();
    let mut session_pointers = Vec::new();
    for p in &memory_pointers {
        if crate::ingestion::crawler::is_decision_path(&p.source) {
            decision_pointers.push(p);
        } else if crate::ingestion::crawler::is_memory_path(&p.source) {
            session_pointers.push(p);
        }
    }

    for dp in decision_pointers.iter().take(3) {
        if let Ok(Some(fetched)) = search.fetch(&dp.id) {
            fetched_tokens += fetched.token_count;
            decision_briefs.push(serde_json::json!({
                "source": dp.source,
                "relevance": dp.relevance,
                "content": fetched.content,
            }));
        }
    }

    let unique_session_files: std::collections::HashSet<&str> =
        session_pointers.iter().map(|p| p.source.as_str()).collect();
    let session_briefs: Vec<serde_json::Value> = session_pointers
        .iter()
        .take(5)
        .map(|sp| {
            serde_json::json!({
                "source": sp.source,
                "relevance": sp.relevance,
                "summary": sp.summary,
            })
        })
        .collect();

    let has_prior_work = !decision_briefs.is_empty() || !session_briefs.is_empty();
    let avoidance_estimate = if has_prior_work {
        fetched_tokens.saturating_mul(RECALL_AVOIDANCE_MULTIPLIER)
            + (unique_session_files.len() as u64).saturating_mul(SESSION_HIT_AVOIDANCE_ESTIMATE)
    } else {
        0
    };

    let pointer_tokens: u64 = memory_pointers
        .iter()
        .chain(related_code.iter())
        .map(Pointer::estimate_token_count)
        .sum();
    let tokens_saved = avoidance_estimate.saturating_sub(pointer_tokens + fetched_tokens);

    let memory_hits = (decision_pointers.len() + session_pointers.len()) as u64;
    let acct = Accountant::new_with_conn(conn, engine.project_id(), engine.session_id());
    acct.record_query_with_memory(
        &format!("recall:{query}"),
        pointer_tokens,
        fetched_tokens,
        avoidance_estimate,
        memory_hits,
    )?;

    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "query": query,
        "has_prior_work": has_prior_work,
        "tokens_saved":  tokens_saved,
        "decisions": decision_briefs,
        "related_sessions": session_briefs,
        "related_code": related_code.iter().take(5).map(|cp| serde_json::json!({
            "source": cp.source,
            "relevance": cp.relevance,
            "summary": cp.summary,
        })).collect::<Vec<_>>(),
        "recommendation": if !decision_briefs.is_empty() {
            "Decision doc(s) found. Read the 'Next Steps (Untried)' section and continue from there."
        } else if !session_briefs.is_empty() {
            "Recent sessions found. Review the summaries to understand context, then proceed."
        } else {
            "No direct match in memory. Proceed with fresh investigation."
        }
    }))?)
}

fn recall_memory_search(
    engine: &HermesEngine,
    graph: &KnowledgeGraph,
    query: &str,
    top_k: usize,
) -> Result<Vec<Pointer>> {
    let search = SearchEngine::new(graph, engine.search_cache());
    let words: Vec<&str> = query
        .split_whitespace()
        .take(RECALL_MAX_QUERY_WORDS)
        .collect();
    let mut results = search.search(&words.join(" "), top_k, &crate::search::SearchMode::Smart)?;

    results.pointers.retain(|p| {
        crate::ingestion::crawler::is_memory_path(&p.source)
            || crate::ingestion::crawler::is_decision_path(&p.source)
    });
    Ok(results.pointers)
}

fn recall_related_code_search(
    engine: &HermesEngine,
    graph: &KnowledgeGraph,
    query: &str,
    top_k: usize,
) -> Result<Vec<Pointer>> {
    let search = SearchEngine::new(graph, engine.search_cache());
    let words: Vec<&str> = query
        .split_whitespace()
        .take(RECALL_MAX_QUERY_WORDS)
        .collect();
    let mut results = search.search(
        &words.join(" "),
        top_k * 2,
        &crate::search::SearchMode::Smart,
    )?;

    results.pointers.retain(|p| {
        !crate::ingestion::crawler::is_memory_path(&p.source)
            && !crate::ingestion::crawler::is_decision_path(&p.source)
    });
    results.pointers.truncate(top_k);
    Ok(results.pointers)
}
