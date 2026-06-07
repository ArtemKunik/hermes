use anyhow::Result;
use rusqlite::params;
use std::time::Duration;

use super::{time_filter_clause, Accountant, CumulativeStats, MemoryUsageStats};

impl Accountant {
    /// Get memory usage statistics (cumulative or since a duration).
    pub fn get_memory_stats(&self, since: Option<Duration>) -> Result<MemoryUsageStats> {
        self.with_conn(|conn| {
            let time_clause = time_filter_clause(since);

            let sql = format!(
                "SELECT COUNT(*) FROM memory_stats
                 WHERE project_id = ?1 AND event_type = 'session_saved'{time_clause}"
            );
            let sessions_saved: u64 =
                conn.query_row(&sql, params![self.project_id], |r| r.get(0))?;

            let sql = format!(
                "SELECT COUNT(*),
                        COALESCE(SUM(CASE WHEN memory_hits > 0 THEN 1 ELSE 0 END), 0),
                        COALESCE(SUM(memory_hits), 0)
                 FROM accounting WHERE project_id = ?1{time_clause}"
            );
            let (total_searches, searches_with_hits, total_hits): (u64, u64, u64) = conn
                .query_row(&sql, params![self.project_id], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })?;

            let search_hit_rate = if total_searches > 0 {
                (searches_with_hits as f64 / total_searches as f64) * 100.0
            } else {
                0.0
            };

            let sql = format!(
                "SELECT COUNT(*),
                        COALESCE(SUM(CASE WHEN memory_hits > 0 THEN 1 ELSE 0 END), 0),
                        COALESCE(SUM(CASE WHEN memory_hits > 0
                            THEN traditional_est - pointer_tokens - fetched_tokens ELSE 0 END), 0)
                 FROM accounting
                 WHERE project_id = ?1
                   AND query_text LIKE 'recall:%'{time_clause}"
            );
            let (total_recalls, recall_hits, recall_avoidance): (u64, u64, i64) =
                conn.query_row(&sql, params![self.project_id], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })?;

            let recall_hit_rate = if total_recalls > 0 {
                (recall_hits as f64 / total_recalls as f64) * 100.0
            } else {
                0.0
            };

            Ok(MemoryUsageStats {
                sessions_saved,
                searches_with_memory_hits: searches_with_hits,
                total_memory_hits: total_hits,
                total_searches,
                memory_hit_rate_pct: search_hit_rate,
                total_recalls,
                recall_hits,
                recall_hit_rate_pct: recall_hit_rate,
                recall_avoidance_tokens_saved: recall_avoidance.max(0) as u64,
            })
        })
    }

    pub fn get_cumulative_stats(&self) -> Result<CumulativeStats> {
        self.get_stats_since(None)
    }

    /// Returns stats scoped to the current session (process invocation) only.
    pub fn get_session_stats(&self) -> Result<CumulativeStats> {
        self.with_conn(|conn| {
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
        })
    }

    /// Task 2.3: Filter stats to a time window.
    /// `since` → only include rows where `created_at >= now - since`.
    /// `None` → all-time (backward compat with `get_cumulative_stats`).
    pub fn get_stats_since(&self, since: Option<Duration>) -> Result<CumulativeStats> {
        self.with_conn(|conn| {
            let time_clause = time_filter_clause(since);

            let sql = format!(
                "SELECT COUNT(*),
                        COALESCE(SUM(pointer_tokens), 0),
                        COALESCE(SUM(fetched_tokens), 0),
                        COALESCE(SUM(traditional_est), 0)
                 FROM accounting WHERE project_id = ?1{time_clause}"
            );

            let mut stmt = conn.prepare(&sql)?;
            let stats = stmt.query_row(params![self.project_id], |row| {
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
        })
    }

    /// Returns per-tool token savings aggregated over all time (or a `since` window).
    /// Each entry: `(tool_name, query_count, tokens_saved)`.
    pub fn get_stats_by_tool(&self, since: Option<Duration>) -> Result<Vec<(String, u64, u64)>> {
        self.with_conn(|conn| {
            let time_clause = time_filter_clause(since);
            let sql = format!(
                "SELECT \
                   CASE \
                      WHEN query_text LIKE 'recall:%'          THEN 'recall' \
                      WHEN query_text LIKE 'repo_map:%'         THEN 'repo_map' \
                      WHEN query_text LIKE 'impact:%'           THEN 'impact_analysis' \
                      WHEN query_text LIKE 'scan_duplicates:%'  THEN 'scan_duplicates' \
                      WHEN query_text LIKE 'match_skills:%'     THEN 'match_skills' \
                      WHEN query_text LIKE 'fetch_skill:%'      THEN 'fetch_skill' \
                      WHEN query_text LIKE 'validate_symbols:%' THEN 'validate_symbols' \
                      WHEN query_text LIKE 'validate_env:%'     THEN 'validate_env' \
                      ELSE 'search_fetch' \
                    END as tool, \
                    COUNT(*) as queries, \
                    COALESCE(SUM(CASE \
                      WHEN traditional_est > pointer_tokens + fetched_tokens \
                      THEN traditional_est - pointer_tokens - fetched_tokens \
                      ELSE 0 END), 0) as tokens_saved \
                  FROM accounting WHERE project_id = ?1{time_clause} \
                  GROUP BY tool ORDER BY tokens_saved DESC"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt
                .query_map(params![self.project_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, u64>(1)?,
                        row.get::<_, u64>(2)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// Build a qualitative impact summary from cumulative accounting + memory data.
    pub fn get_impact_summary(&self) -> Result<serde_json::Value> {
        let stats = self.get_cumulative_stats()?;
        let by_tool = self.get_stats_by_tool(None)?;
        let memory = self.get_memory_stats(None)?;

        let tokens_saved = stats.cumulative_savings_tokens;
        let files_avoided = tokens_saved / 200;
        let context_windows = tokens_saved / 8_000;
        let cost_usd = (tokens_saved as f64) / 1_000.0 * 0.03;

        let top_tool = by_tool
            .iter()
            .max_by_key(|(_, _, saved)| saved)
            .map(|(tool, _, _)| tool.as_str())
            .unwrap_or("none");

        let headline = if stats.total_queries > 0 {
            format!(
                "Hermes handled {} queries at {:.0}% compression, \
                 avoiding ~{files_avoided} source-file reads \
                 (est. ${:.2} in LLM input cost)",
                stats.total_queries, stats.cumulative_savings_pct, cost_usd,
            )
        } else {
            "No queries recorded yet.".to_string()
        };

        const TOOL_DESCRIPTIONS: &[(&str, &str)] = &[
            ("search_fetch",    "code navigation queries — files read via compact pointers, not full content"),
            ("recall",          "prior-work recalls — decisions and session history retrieved without re-investigation"),
            ("repo_map",        "repo overview requests — full symbol index surfaced without reading every file"),
            ("impact_analysis", "blast-radius analyses — change impact traced through the dependency graph"),
            ("scan_duplicates", "duplicate scans — embedding similarity checked across the full symbol index"),
            ("match_skills",    "skill lookups — relevant skills ranked without scanning skill directories"),
            ("fetch_skill",     "skill fetches — skill content retrieved via indexed path"),
            ("validate_symbols","symbol validations — existence confirmed against the indexed knowledge graph"),
            ("validate_env",    "env-var validations — variable validity checked against the config registry"),
        ];

        let by_tool_narrative: Vec<serde_json::Value> = by_tool
            .iter()
            .map(|(tool, queries, saved)| {
                let desc = TOOL_DESCRIPTIONS
                    .iter()
                    .find(|(t, _)| *t == tool.as_str())
                    .map(|(_, d)| *d)
                    .unwrap_or("queries handled by Hermes");
                serde_json::json!({
                    "tool": tool,
                    "queries": queries,
                    "tokens_saved": saved,
                    "narrative": format!("{queries} {desc}"),
                })
            })
            .collect();

        Ok(serde_json::json!({
            "headline": headline,
            "files_reading_avoided": files_avoided,
            "context_windows_saved": context_windows,
            "cost_usd_saved": format!("~${:.2}", cost_usd),
            "re_investigations_avoided": memory.recall_hits,
            "sessions_committed_to_memory": memory.sessions_saved,
            "top_savings_tool": top_tool,
            "by_tool_narrative": by_tool_narrative,
        }))
    }
}
