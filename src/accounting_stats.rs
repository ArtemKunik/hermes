use anyhow::Result;
use rusqlite::params;
use std::time::Duration;

use super::{time_filter_clause, Accountant, CumulativeStats, MemoryUsageStats};

impl Accountant {
    pub fn get_memory_stats(&self, since: Option<Duration>) -> Result<MemoryUsageStats> {
        self.with_conn(|conn| {
            let time_clause = time_filter_clause(since);

            let sql = format!(
                "SELECT COUNT(*) FROM memory_stats
                 WHERE project_id = ?1 AND event_type = 'session_saved'{time_clause}"
            );
            let sessions_saved: u64 = conn.query_row(&sql, params![self.project_id], |r| r.get(0))?;

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
            } else { 0.0 };

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
            } else { 0.0 };

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

    pub fn get_stats_by_tool(&self, since: Option<Duration>) -> Result<Vec<(String, u64, u64)>> {
        self.with_conn(|conn| {
            let time_clause = time_filter_clause(since);
            let sql = format!(
                "SELECT \
                   CASE \
                      WHEN query_text LIKE 'recall:%'          THEN 'recall' \
                      WHEN query_text LIKE 'repo_map:%'         THEN 'repo_map' \
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

    pub fn get_impact_summary(&self) -> Result<serde_json::Value> {
        let stats = self.get_cumulative_stats()?;
        let by_tool = self.get_stats_by_tool(None)?;
        let memory = self.get_memory_stats(None)?;

        let tokens_saved = stats.cumulative_savings_tokens;
        let files_avoided = tokens_saved / 200;
        let cost_usd = (tokens_saved as f64) / 1_000.0 * 0.03;

        let top_tool = by_tool
            .iter()
            .max_by_key(|(_, _, saved)| saved)
            .map(|(tool, _, _)| tool.as_str())
            .unwrap_or("none");

        let headline = if stats.total_queries > 0 {
            format!(
                "Hermes handled {} queries at {:.0}% compression, \
                 avoiding ~{files_avoided} source-file reads (est. ${cost_usd:.2} in LLM input cost)",
                stats.total_queries, stats.cumulative_savings_pct,
            )
        } else {
            "No queries recorded yet.".to_string()
        };

        const TOOL_DESCRIPTIONS: &[(&str, &str)] = &[
            ("search_fetch", "code navigation queries — files read via compact pointers, not full content"),
            ("recall",       "prior-work recalls — decisions and session history retrieved without re-investigation"),
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
            "cost_usd_saved": format!("~${cost_usd:.2}"),
            "re_investigations_avoided": memory.recall_hits,
            "sessions_committed_to_memory": memory.sessions_saved,
            "top_savings_tool": top_tool,
            "by_tool_narrative": by_tool_narrative,
        }))
    }
}
