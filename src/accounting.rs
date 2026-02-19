use anyhow::Result;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CumulativeStats {
    pub total_queries: u64,
    pub total_pointer_tokens: u64,
    pub total_fetched_tokens: u64,
    pub total_traditional_estimate: u64,
    pub cumulative_savings_tokens: u64,
    pub cumulative_savings_pct: f64,
}

pub struct Accountant {
    db: Arc<Mutex<Connection>>,
    project_id: String,
    session_id: String,
}

impl Accountant {
    pub fn new(db: Arc<Mutex<Connection>>, project_id: &str, session_id: &str) -> Self {
        Self {
            db,
            project_id: project_id.to_string(),
            session_id: session_id.to_string(),
        }
    }

    pub fn record_query(
        &self,
        query_text: &str,
        pointer_tokens: u64,
        fetched_tokens: u64,
        traditional_estimate: u64,
    ) -> Result<()> {
        let conn = self.db.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        conn.execute(
            "INSERT INTO accounting (project_id, session_id, query_text, pointer_tokens, fetched_tokens, traditional_est)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                self.project_id,
                self.session_id,
                query_text,
                pointer_tokens as i64,
                fetched_tokens as i64,
                traditional_estimate as i64,
            ],
        )?;
        Ok(())
    }

    pub fn get_cumulative_stats(&self) -> Result<CumulativeStats> {
        self.get_stats_since(None)
    }

    pub fn get_stats_since(&self, since: Option<Duration>) -> Result<CumulativeStats> {
        let conn = self.db.lock().map_err(|e| anyhow::anyhow!("{e}"))?;

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
        let stats = stmt.query_row(rusqlite::params_from_iter(params_values.iter()), |row| {
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
        })?;
        Ok(stats)
    }

    pub fn get_session_stats(&self) -> Result<CumulativeStats> {
        let conn = self.db.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut stmt = conn.prepare(
            "SELECT COUNT(*),
                    COALESCE(SUM(pointer_tokens), 0),
                    COALESCE(SUM(fetched_tokens), 0),
                    COALESCE(SUM(traditional_est), 0)
             FROM accounting WHERE project_id = ?1 AND session_id = ?2",
        )?;
        let stats = stmt.query_row(params![self.project_id, self.session_id], |row| {
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
        })?;
        Ok(stats)
    }
}

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

        let stats = acct.get_stats_since(Some(Duration::from_secs(3600))).unwrap();
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

    #[test]
    fn parse_since_invalid_returns_none() {
        assert!(parse_since_duration("yesterday").is_none());
        assert!(parse_since_duration("").is_none());
        assert!(parse_since_duration("abc").is_none());
    }

    #[test]
    fn parse_since_1h() {
        let dur = parse_since_duration("1h").unwrap();
        assert_eq!(dur.as_secs(), 3600);
    }

    #[test]
    fn session_stats_are_isolated_by_session_id() {
        let engine = HermesEngine::in_memory("test-session-iso").unwrap();
        let acct_a = Accountant::new(engine.db().clone(), "test-session-iso", "session-A");
        let acct_b = Accountant::new(engine.db().clone(), "test-session-iso", "session-B");

        acct_a.record_query("q1", 100, 0, 1000).unwrap();
        acct_b.record_query("q2", 200, 0, 2000).unwrap();

        let stats_a = acct_a.get_session_stats().unwrap();
        let stats_b = acct_b.get_session_stats().unwrap();

        assert_eq!(stats_a.total_queries, 1);
        assert_eq!(stats_a.total_pointer_tokens, 100);
        assert_eq!(stats_b.total_queries, 1);
        assert_eq!(stats_b.total_pointer_tokens, 200);

        // cumulative covers both sessions
        let all = acct_a.get_cumulative_stats().unwrap();
        assert_eq!(all.total_queries, 2);
    }

    #[test]
    fn savings_pct_zero_when_no_traditional_estimate() {
        let engine = HermesEngine::in_memory("test-zero-est").unwrap();
        let acct = Accountant::new(engine.db().clone(), "test-zero-est", engine.session_id());
        acct.record_query("q", 50, 0, 0).unwrap();
        let stats = acct.get_cumulative_stats().unwrap();
        assert_eq!(stats.cumulative_savings_pct, 0.0);
    }
}

