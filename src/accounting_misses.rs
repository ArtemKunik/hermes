use anyhow::Result;
use rusqlite::params;
use std::time::Duration;

use super::{time_filter_clause, Accountant, SearchMissRow};

impl Accountant {
    /// Record a search that returned zero pointers for post-mortem analysis.
    pub fn record_search_miss(
        &self,
        query: &str,
        effective_query: Option<&str>,
        goal: Option<&str>,
        source: &str,
    ) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO search_misses (project_id, session_id, query, effective_query, goal, source)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    self.project_id,
                    self.session_id,
                    query,
                    effective_query,
                    goal,
                    source,
                ],
            )?;
            Ok(())
        })
    }

    /// Return the `limit` most recent zero-result search queries.
    pub fn query_search_misses(
        &self,
        limit: usize,
        since: Option<Duration>,
    ) -> Result<Vec<SearchMissRow>> {
        self.with_conn(|conn| {
            let time_clause = time_filter_clause(since);
            let sql = format!(
                "SELECT id, session_id, query, effective_query, goal, source, created_at
                 FROM search_misses
                 WHERE project_id = ?1{time_clause}
                 ORDER BY created_at DESC
                 LIMIT ?2"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt
                .query_map(params![self.project_id, limit as i64], |row| {
                    Ok(SearchMissRow {
                        id: row.get(0)?,
                        session_id: row.get(1)?,
                        query: row.get(2)?,
                        effective_query: row.get(3)?,
                        goal: row.get(4)?,
                        source: row.get(5)?,
                        created_at: row.get(6)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// Return the top missed queries ranked by frequency (most repeated first).
    pub fn top_missed_queries(
        &self,
        limit: usize,
        since: Option<Duration>,
    ) -> Result<Vec<(String, u64)>> {
        self.with_conn(|conn| {
            let time_clause = time_filter_clause(since);
            let sql = format!(
                "SELECT query, COUNT(*) as cnt
                 FROM search_misses
                 WHERE project_id = ?1{time_clause}
                 GROUP BY query
                 ORDER BY cnt DESC
                 LIMIT ?2"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt
                .query_map(params![self.project_id, limit as i64], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// Determine whether a battery-change review is due.
    pub fn battery_review_due(&self, threshold: u64, days: u64) -> Result<bool> {
        self.with_conn(|conn| {
            let last_sql = "SELECT MAX(created_at) FROM memory_stats
                            WHERE project_id = ?1 AND event_type = 'battery_review'";
            let last_ts: Option<String> =
                conn.query_row(last_sql, params![self.project_id], |r| r.get(0))?;

            let count_sql = if last_ts.is_some() {
                "SELECT COUNT(*) FROM memory_stats
                 WHERE project_id = ?1 AND event_type = 'session_saved'
                   AND created_at > ?2"
            } else {
                "SELECT COUNT(*) FROM memory_stats
                 WHERE project_id = ?1 AND event_type = 'session_saved'"
            };
            let session_count: u64 = if let Some(ref ts) = last_ts {
                conn.query_row(count_sql, params![self.project_id, ts], |r| r.get(0))?
            } else {
                conn.query_row(count_sql, params![self.project_id], |r| r.get(0))?
            };

            if session_count >= threshold {
                return Ok(true);
            }
            if let Some(ts) = last_ts {
                if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&ts) {
                    let age = chrono::Utc::now().signed_duration_since(dt.with_timezone(&chrono::Utc));
                    if age.num_days() as u64 >= days {
                        return Ok(true);
                    }
                }
            }
            Ok(false)
        })
    }
}
