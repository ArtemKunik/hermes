// ChartApp/hermes-engine/src/accounting.rs
use anyhow::Result;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[path = "accounting_misses.rs"]
mod accounting_misses;
#[path = "accounting_stats.rs"]
mod accounting_stats;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CumulativeStats {
    pub total_queries: u64,
    pub total_pointer_tokens: u64,
    pub total_fetched_tokens: u64,
    pub total_traditional_estimate: u64,
    pub cumulative_savings_tokens: u64,
    pub cumulative_savings_pct: f64,
}

/// Memory-specific usage statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryUsageStats {
    /// Total hermes_remember calls (sessions saved).
    pub sessions_saved: u64,
    /// Total search queries that returned at least one memory result.
    pub searches_with_memory_hits: u64,
    /// Total memory pointer results returned across all searches.
    pub total_memory_hits: u64,
    /// Total search queries overall (denominator for hit rate).
    pub total_searches: u64,
    /// Percentage of searches that recalled memory.
    pub memory_hit_rate_pct: f64,
    /// Total hermes_recall invocations overall.
    pub total_recalls: u64,
    /// Total hermes_recall invocations that found prior work.
    pub recall_hits: u64,
    /// Percentage of recalls that found prior work.
    pub recall_hit_rate_pct: f64,
    /// Estimated tokens saved by recalling prior work instead of re-investigating.
    ///
    /// Computed as: fetched decision content tokens × RECALL_AVOIDANCE_MULTIPLIER
    /// plus a flat estimate per session hit. Represents work the agent avoided
    /// by consulting memory rather than re-reading source files and session history.
    pub recall_avoidance_tokens_saved: u64,
}

/// A single recorded search-miss: a query that returned zero pointers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchMissRow {
    pub id: i64,
    pub session_id: String,
    pub query: String,
    pub effective_query: Option<String>,
    pub goal: Option<String>,
    pub source: String,
    pub created_at: String,
}

pub struct Accountant {
    db: AcctConn,
    project_id: String,
    session_id: String,
}

enum AcctConn {
    Shared(Arc<Mutex<Connection>>),
    Borrowed(*const Connection),
}

// Safety: Accountant is used in threads, but we ensure the lifetime of
// the borrowed connection exceeds the graph during execute_tool_call.
unsafe impl Send for AcctConn {}
unsafe impl Sync for AcctConn {}

impl Accountant {
    pub fn new(db: Arc<Mutex<Connection>>, project_id: &str, session_id: &str) -> Self {
        Self {
            db: AcctConn::Shared(db),
            project_id: project_id.to_string(),
            session_id: session_id.to_string(),
        }
    }

    /// TRACK-066: Create an accountant from a raw connection (read-only isolation).
    pub fn from_conn(conn: &Connection, project_id: &str, session_id: &str) -> Self {
        Self {
            db: AcctConn::Borrowed(conn as *const Connection),
            project_id: project_id.to_string(),
            session_id: session_id.to_string(),
        }
    }

    pub fn new_with_conn(conn: &Connection, project_id: &str, session_id: &str) -> Self {
        Self::from_conn(conn, project_id, session_id)
    }

    fn with_conn<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&Connection) -> Result<R>,
    {
        match &self.db {
            AcctConn::Shared(arc) => {
                let conn = arc.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
                f(&conn)
            }
            AcctConn::Borrowed(ptr) => {
                // Safety: ptr is valid for the duration of the tool call.
                let conn = unsafe { &**ptr };
                f(conn)
            }
        }
    }

    pub fn record_query(
        &self,
        query_text: &str,
        pointer_tokens: u64,
        fetched_tokens: u64,
        traditional_estimate: u64,
    ) -> Result<()> {
        self.record_query_with_memory(
            query_text,
            pointer_tokens,
            fetched_tokens,
            traditional_estimate,
            0,
        )
    }

    /// Record a query with explicit memory hit count.
    pub fn record_query_with_memory(
        &self,
        query_text: &str,
        pointer_tokens: u64,
        fetched_tokens: u64,
        traditional_estimate: u64,
        memory_hits: u64,
    ) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO accounting (project_id, session_id, query_text, pointer_tokens, fetched_tokens, traditional_est, memory_hits)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    self.project_id,
                    self.session_id,
                    query_text,
                    pointer_tokens as i64,
                    fetched_tokens as i64,
                    traditional_estimate as i64,
                    memory_hits as i64,
                ],
            )?;
            Ok(())
        })
    }

    /// Record a memory event (session saved, decision saved, etc).
    pub fn record_memory_event(
        &self,
        event_type: &str,
        topic: Option<&str>,
        file_path: Option<&str>,
        tags: Option<&str>,
    ) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO memory_stats (project_id, session_id, event_type, topic, file_path, tags)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    self.project_id,
                    self.session_id,
                    event_type,
                    topic,
                    file_path,
                    tags
                ],
            )?;
            Ok(())
        })
    }
}

/// Build an AND clause for time filtering. Empty string = no filter.
pub(crate) fn time_filter_clause(since: Option<Duration>) -> String {
    match since {
        Some(dur) => format!(
            " AND created_at >= datetime('now', '-{} seconds')",
            dur.as_secs()
        ),
        None => String::new(),
    }
}

/// Task 2.3: Parses --since flag values into a Duration.
/// Accepted: "24h", "7d", "30d", "all" (→ None = no filter).
pub fn parse_since_duration(s: &str) -> Option<Duration> {
    match s.trim().to_lowercase().as_str() {
        "all" => None,
        s if s.ends_with('h') => {
            let hours: u64 = s.trim_end_matches('h').parse().ok()?;
            Some(Duration::from_secs(hours * 3600))
        }
        s if s.ends_with('d') => {
            let days: u64 = s.trim_end_matches('d').parse().ok()?;
            Some(Duration::from_secs(days * 86400))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HermesEngine;

    #[test]
    fn record_and_aggregate_queries() {
        let engine = HermesEngine::in_memory("test").unwrap();
        let acct = Accountant::new(engine.db().clone(), "test", engine.session_id());

        acct.record_query("find main function", 300, 0, 15000)
            .unwrap();
        acct.record_query("search currency service", 250, 1200, 12000)
            .unwrap();

        let stats = acct.get_cumulative_stats().unwrap();
        assert_eq!(stats.total_queries, 2);
        assert_eq!(stats.total_pointer_tokens, 550);
        assert_eq!(stats.total_fetched_tokens, 1200);
        assert_eq!(stats.total_traditional_estimate, 27000);
        assert_eq!(stats.cumulative_savings_tokens, 25250);
        assert!(stats.cumulative_savings_pct > 90.0);

        let session = acct.get_session_stats().unwrap();
        assert_eq!(session.total_queries, 2);
        assert_eq!(session.cumulative_savings_tokens, 25250);
    }

    #[test]
    fn empty_stats_returns_zeros() {
        let engine = HermesEngine::in_memory("test").unwrap();
        let acct = Accountant::new(engine.db().clone(), "test", engine.session_id());

        let stats = acct.get_cumulative_stats().unwrap();
        assert_eq!(stats.total_queries, 0);
        assert_eq!(stats.cumulative_savings_pct, 0.0);

        let session = acct.get_session_stats().unwrap();
        assert_eq!(session.total_queries, 0);
    }

    #[test]
    fn get_stats_since_returns_only_recent_rows() {
        let engine = HermesEngine::in_memory("test-since").unwrap();
        let acct = Accountant::new(engine.db().clone(), "test-since", engine.session_id());

        acct.record_query("q1", 100, 0, 5000).unwrap();

        let stats = acct
            .get_stats_since(Some(Duration::from_secs(3600)))
            .unwrap();
        assert_eq!(stats.total_queries, 1);
    }

    #[test]
    fn parse_since_24h() {
        let dur = parse_since_duration("24h").unwrap();
        assert_eq!(dur.as_secs(), 86400);
    }

    #[test]
    fn parse_since_7d() {
        let dur = parse_since_duration("7d").unwrap();
        assert_eq!(dur.as_secs(), 7 * 86400);
    }

    #[test]
    fn parse_since_all_returns_none() {
        assert!(parse_since_duration("all").is_none());
    }
}
