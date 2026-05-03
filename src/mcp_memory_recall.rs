// COHERENCE: extracted recall helpers to reduce mcp_memory.rs size for TRACK-046 remediation
// Extracted recall helpers from mcp_memory.rs to reduce file size for SIZE-001

const RECALL_AVOIDANCE_MULTIPLIER: u64 = 7;

/// Flat avoidance credit per session pointer hit where no full fetch was done.
/// Represents the cost of manually re-reading one raw session file to find relevant context.
const SESSION_HIT_AVOIDANCE_ESTIMATE: u64 = 1_500;

/// Search memory for related sessions and decisions, returning a structured
/// context brief with previous attempts, outcomes, and recommended next steps.
pub fn tool_recall(engine: &crate::HermesEngine, query: &str) -> anyhow::Result<String> {
    anyhow::ensure!(!query.is_empty(), "hermes_recall requires 'query'");

    // Use the dedicated read connection so long FTS scans here don't block
    // write operations (accounting, auto-index) on the main db mutex.
    let graph = crate::graph::KnowledgeGraph::new(engine.read_db().clone(), engine.project_id());
    let search = crate::search::SearchEngine::new(&graph, engine.search_cache());
    let memory_pointers = recall_memory_search(&graph, query, 20)?;
    let related_code = recall_related_code_search(&graph, query, 5)?;

    let mut decision_pointers = Vec::new();
    let mut session_pointers = Vec::new();
    for p in &memory_pointers {
        if crate::ingestion::crawler::is_decision_path(&p.source) {
            decision_pointers.push(p);
        } else if crate::ingestion::crawler::is_memory_path(&p.source) {
            session_pointers.push(p);
        }
    }

    let mut decision_briefs: Vec<serde_json::Value> = Vec::new();
    let mut fetched_tokens: u64 = 0;
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
        .map(crate::pointer::Pointer::estimate_token_count)
        .sum();
    let tokens_saved = avoidance_estimate.saturating_sub(pointer_tokens + fetched_tokens);

    let memory_hits = (decision_pointers.len() + session_pointers.len()) as u64;
    let acct = crate::accounting::Accountant::new(
        engine.db().clone(),
        engine.project_id(),
        engine.session_id(),
    );
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
            "Related session(s) found but no decision doc. Consider creating memory/decisions/<topic>.md before proceeding."
        } else {
            "No prior work found in memory. Proceed with fresh implementation."
        }
    }))?)
}

const RECALL_MAX_QUERY_WORDS: usize = 10;

fn recall_memory_search(graph: &crate::graph::KnowledgeGraph, query: &str, limit: usize) -> anyhow::Result<Vec<crate::pointer::Pointer>> {
    let words = recall_extract_query_terms(query);
    if words.is_empty() {
        return Ok(Vec::new());
    }

    let phrase_query = format!("\"{}\"", words.join(" "));
    let phrase = run_memory_fts_query(graph, &phrase_query, limit)?;
    if phrase.len() >= 3 {
        return Ok(phrase);
    }

    let and_query = words
        .iter()
        .map(|w| format!("\"{w}\"*"))
        .collect::<Vec<_>>()
        .join(" AND ");
    let and_res = run_memory_fts_query(graph, &and_query, limit)?;
    if and_res.len() >= 3 {
        return Ok(and_res);
    }

    let or_query = words
        .iter()
        .map(|w| format!("\"{w}\""))
        .collect::<Vec<_>>()
        .join(" OR ");
    run_memory_fts_query(graph, &or_query, limit)
}

fn run_memory_fts_query(
    graph: &crate::graph::KnowledgeGraph,
    fts_query: &str,
    limit: usize,
) -> anyhow::Result<Vec<crate::pointer::Pointer>> {
    // Scope the FTS MATCH to nodes whose file_path column contains one of the
    // memory directory tokens (sessions, decisions, handover).  This uses the
    // FTS5 column-filter syntax — `{file_path}: expr` — which is evaluated
    // inside the FTS index and avoids the previous `LIKE '%memory%'` leading-
    // wildcard scan that ran over every FTS hit before the LIMIT was reached.
    // Parentheses around the content part force correct AND > OR precedence so
    // that OR-fallback queries are scoped to memory nodes too.
    let memory_scoped_query = format!(
        "{{file_path}}: (sessions OR decisions OR handover) AND ({})",
        fts_query
    );
    let conn = graph.db().lock().map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut stmt = conn.prepare(
        "SELECT n.id, n.name, n.file_path, n.start_line, n.end_line, n.summary, n.node_type, n.content_tokens,
                bm25(fts_content) as rank
         FROM fts_content f
         JOIN nodes n ON n.id = f.node_id
         WHERE f.project_id = ?1
           AND n.project_id = ?1
           AND fts_content MATCH ?2
         ORDER BY rank
         LIMIT ?3",
    )?;

    let mut rows = stmt.query(rusqlite::params![graph.project_id(), memory_scoped_query, limit as i64])?;
    let mut pointers = Vec::new();
    while let Some(row) = rows.next()? {
        let file_path: Option<String> = row.get(2)?;
        let source = file_path.unwrap_or_default();
        if !crate::ingestion::crawler::is_memory_path(&source) {
            continue;
        }
        let pointer = crate::pointer::Pointer {
            id: row.get(0)?,
            source,
            chunk: row.get::<_, String>(1)?,
            lines: format!(
                "{}-{}",
                row.get::<_, Option<i64>>(3)?.unwrap_or(0),
                row.get::<_, Option<i64>>(4)?.unwrap_or(0)
            ),
            relevance: normalize_bm25_score(row.get::<_, f64>(8)?),
            summary: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
            node_type: row.get::<_, String>(6)?,
            last_modified: None,
            content_tokens: row.get::<_, Option<i64>>(7)?.map(|v| v as u64),
            object_type: None,
        };
        pointers.push(pointer);
    }
    Ok(pointers)
}

fn recall_related_code_search(
    graph: &crate::graph::KnowledgeGraph,
    query: &str,
    limit: usize,
) -> anyhow::Result<Vec<crate::pointer::Pointer>> {
    let query_term = recall_extract_query_terms(query)
        .into_iter()
        .next()
        .unwrap_or_default();
    if query_term.is_empty() {
        return Ok(Vec::new());
    }

    let mut pointers = Vec::new();
    for node in graph.literal_search_by_name(&query_term)? {
        let file_path = node.file_path.clone().unwrap_or_default();
        if crate::ingestion::crawler::is_memory_path(&file_path) {
            continue;
        }
        pointers.push(pointer_from_node(&node, 0.6));
        if pointers.len() >= limit {
            break;
        }
    }
    Ok(pointers)
}

fn recall_extract_query_terms(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .filter(|w| !matches!(w.to_uppercase().as_str(), "AND" | "OR" | "NOT" | "NEAR"))
        .map(|w| w.trim_matches('"').replace('"', "").trim().to_string())
        .filter(|w| !w.is_empty())
        .take(RECALL_MAX_QUERY_WORDS)
        .collect()
}

fn normalize_bm25_score(rank: f64) -> f64 {
    let abs_rank = rank.abs();
    if abs_rank < 0.001 {
        return 0.5;
    }
    (1.0 - 1.0 / (1.0 + abs_rank)).min(1.0)
}

fn pointer_from_node(node: &crate::graph::Node, relevance: f64) -> crate::pointer::Pointer {
    crate::pointer::Pointer {
        id: node.id.clone(),
        source: node.file_path.clone().unwrap_or_default(),
        chunk: node.name.clone(),
        lines: format!(
            "{}-{}",
            node.start_line.unwrap_or(0),
            node.end_line.unwrap_or(0)
        ),
        relevance,
        summary: node.summary.clone().unwrap_or_default(),
        node_type: node.node_type.as_str().to_string(),
        last_modified: None,
        content_tokens: node.content_tokens,
        object_type: node.object_type.clone(),
    }
}
