#[path = "accounting_misses.rs"]
mod accounting_misses;
#[path = "accounting_stats.rs"]
mod accounting_stats;

use anyhow::Result;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::lock_ext::LockExt;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CumulativeStats {
    pub total_queries: u64,
    pub total_pointer_tokens: u64,
    pub total_fetched_tokens: u64,
    pub total_traditional_estimate: u64,
    pub cumulative_savings_tokens: u64,
    pub cumulative_savings_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryUsageStats {
    pub sessions_saved: u64,
    pub searches_with_memory_hits: u64,
    pub total_memory_hits: u64,
    pub total_searches: u64,
    pub memory_hit_rate_pct: f64,
    pub total_recalls: u64,
    pub recall_hits: u64,
    pub recall_hit_rate_pct: f64,
    pub recall_avoidance_tokens_saved: u64,
}

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

    pub fn from_conn(conn: &Connection, project_id: &str, session_id: &str) -> Self {
        Self {
            db: AcctConn::Borrowed(conn as *const Connection),
            project_id: project_id.to_string(),
            session_id: session_id.to_string(),
        }
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
        self.record_query_with_memory(query_text, pointer_tokens, fetched_tokens, traditional_estimate, 0)
    }

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

    pub fn get_stats_since(&self, since: Option<Duration>) -> Result<CumulativeStats> {
        let conn = self.db.lock_ctx("get_stats_since")?;

        let (query, params_values): (String, Vec<String>) = if let Some(dur) = since {
            let secs = dur.as_secs() as i64;
            (
                format!(
                    "SELECT COUNT(*),
                            COALESCE(SUM(pointer_tokens), 0),
                            COALESCE(SUM(fetched_tokens), 0),
                            COALESCE(SUM(traditional_est), 0)
                     FROM accounting
                     WHERE project_id = ?1
                       AND created_at >= datetime('now', '-{} seconds')",
                    secs
                ),
                vec![self.project_id.clone()],
            )
        } else {
            (
                "SELECT COUNT(*),
                        COALESCE(SUM(pointer_tokens), 0),
                        COALESCE(SUM(fetched_tokens), 0),
                        COALESCE(SUM(traditional_est), 0)
                 FROM accounting WHERE project_id = ?1"
                    .to_string(),
                vec![self.project_id.clone()],
            )
        };

        let mut stmt = conn.prepare(&query)?;
        let stats = stmt.query_row(rusqlite::params_from_iter(params_values.iter()), stats_from_row)?;
        Ok(stats)
    }

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
                params![self.project_id, self.session_id, event_type, topic, file_path, tags],
            )?;
            Ok(())
        })
    }
}

    pub fn get_session_stats(&self) -> Result<CumulativeStats> {
        let conn = self.db.lock_ctx("get_session_stats")?;
        let mut stmt = conn.prepare(
            "SELECT COUNT(*),
                    COALESCE(SUM(pointer_tokens), 0),
                    COALESCE(SUM(fetched_tokens), 0),
                    COALESCE(SUM(traditional_est), 0)
             FROM accounting WHERE project_id = ?1 AND session_id = ?2",
        )?;
        let stats = stmt.query_row(params![self.project_id, self.session_id], stats_from_row)?;
        Ok(stats)
    }

    /// Stats for the current calendar day (local time, 00:00–24:00).
    /// More robust than session_stats when a long-running process crosses
    /// midnight, because it uses the SQLite `date('now','localtime')` function
    /// rather than the session_id string that was set at startup.
    pub fn get_today_stats(&self) -> Result<CumulativeStats> {
        let conn = self.db.lock_ctx("get_today_stats")?;
        let mut stmt = conn.prepare(
            "SELECT COUNT(*),
                    COALESCE(SUM(pointer_tokens), 0),
                    COALESCE(SUM(fetched_tokens), 0),
                    COALESCE(SUM(traditional_est), 0)
             FROM accounting
             WHERE project_id = ?1
               AND date(created_at, 'localtime') = date('now', 'localtime')",
        )?;
        let stats = stmt.query_row(params![self.project_id], stats_from_row)?;
        Ok(stats)
    }
}

pub(crate) fn time_filter_clause(since: Option<Duration>) -> String {
    match since {
        Some(dur) => format!(
            " AND created_at >= datetime('now', '-{} seconds')",
            dur.as_secs()
        ),
        None => String::new(),
    }
}

fn stats_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CumulativeStats> {
    let total_queries: u64 = row.get(0)?;
    let ptr_tokens: u64 = row.get(1)?;
    let fetch_tokens: u64 = row.get(2)?;
    let trad_est: u64 = row.get(3)?;
    let actual = ptr_tokens + fetch_tokens;
    let saved = trad_est.saturating_sub(actual);
    let pct = if trad_est > 0 {
        (saved as f64 / trad_est as f64) * 100.0
    } else {
        0.0
    };
    Ok(CumulativeStats {
        total_queries,
        total_pointer_tokens: ptr_tokens,
        total_fetched_tokens: fetch_tokens,
        total_traditional_estimate: trad_est,
        cumulative_savings_tokens: saved,
        cumulative_savings_pct: pct,
    })
}

pub fn parse_since_duration(s: &str) -> Option<Duration> {
    match s.trim().to_lowercase().as_str() {
        "all" => None,
        _ => {
            let s = s.trim();
            let (num_str, unit) = s.split_at(s.len() - 1);
            let num: u64 = num_str.parse().ok()?;
            match unit {
                "h" => Some(Duration::from_secs(num * 3600)),
                "d" => Some(Duration::from_secs(num * 86400)),
                "m" if num_str.len() > 2 => None,
                _ => None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HermesEngine;

    #[test]
    fn record_and_query_accounting() {
        let engine = HermesEngine::in_memory("test-acct").unwrap();
        let acct = Accountant::new(engine.db().clone(), "test-acct", engine.session_id());
        acct.record_query("test query", 100, 50, 5000).unwrap();

        let stats = acct.get_cumulative_stats().unwrap();
        assert_eq!(stats.total_queries, 1);
        assert_eq!(stats.total_pointer_tokens, 100);
        assert_eq!(stats.total_fetched_tokens, 50);
        assert!(stats.cumulative_savings_pct > 0.0);
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
        assert_eq!(stats.total_pointer_tokens, 100);
        assert_eq!(stats.total_fetched_tokens, 50);
        assert!(stats.cumulative_savings_pct > 0.0);
    }

    #[test]
    fn parse_since_hours() {
        let dur = parse_since_duration("24h").unwrap();
        assert_eq!(dur.as_secs(), 86400);
    }

    #[test]
    fn parse_since_days() {
        let dur = parse_since_duration("7d").unwrap();
        assert_eq!(dur.as_secs(), 604800);
    }

    #[test]
    fn parse_since_all_returns_none() {
        assert!(parse_since_duration("all").is_none());
    }

    #[test]
    fn parse_since_invalid_returns_none() {
        assert!(parse_since_duration("abc").is_none());
    }

    #[test]
    fn savings_pct_zero_when_no_traditional_estimate() {
        let engine = HermesEngine::in_memory("test-zero").unwrap();
        let acct = Accountant::new(engine.db().clone(), "test-zero", engine.session_id());
        acct.record_query("q", 50, 0, 0).unwrap();
        let stats = acct.get_cumulative_stats().unwrap();
        assert_eq!(stats.cumulative_savings_pct, 0.0);
    }
}
