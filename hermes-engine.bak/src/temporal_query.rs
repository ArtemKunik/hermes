// tools/hermes-engine/src/temporal_query.rs
//
// Query filters and helpers for temporal fact retrieval.

use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};

use crate::temporal::TemporalFact;

// ── Query filter ─────────────────────────────────────────────────────────

pub struct FactQueryFilter<'a> {
    pub fact_type: Option<&'a str>,
    pub topic: Option<&'a str>,
    pub tags: Option<&'a str>,
    pub repo_id: Option<&'a str>,
    pub limit: usize,
    pub include_expired: bool,
}

pub(crate) fn query_facts_inner(
    conn: &Connection,
    project_id: &str,
    f: &FactQueryFilter,
) -> Result<Vec<TemporalFact>> {
    let mut sql = String::from(
        "SELECT id, project_id, node_id, fact_type, content,
                topic, tags, confidence, valid_from, valid_to,
                superseded_by, source_reference, provenance,
                repo_id, agent_id
         FROM temporal_facts WHERE project_id = ?1",
    );
    let mut pv: Vec<rusqlite::types::Value> = vec![project_id.to_string().into()];
    let mut idx = 2u32;

    if !f.include_expired {
        sql.push_str(" AND (valid_to IS NULL OR valid_to > datetime('now'))");
    }
    if let Some(ft) = f.fact_type {
        sql.push_str(&format!(" AND fact_type = ?{idx}"));
        pv.push(ft.to_string().into());
        idx += 1;
    }
    if let Some(t) = f.topic {
        sql.push_str(&format!(" AND topic = ?{idx}"));
        pv.push(t.to_string().into());
        idx += 1;
    }
    if let Some(tags) = f.tags {
        for tag in tags.split(',').map(str::trim) {
            sql.push_str(&format!(" AND tags LIKE ?{idx}"));
            pv.push(format!("%{tag}%").into());
            idx += 1;
        }
    }
    if let Some(r) = f.repo_id {
        sql.push_str(&format!(" AND repo_id = ?{idx}"));
        pv.push(r.to_string().into());
    }
    sql.push_str(" ORDER BY valid_from DESC LIMIT ?");
    pv.push((f.limit as i64).into());

    let mut stmt = conn.prepare(&sql)?;
    let now_str = Utc::now().to_rfc3339();
    let rows = stmt.query_map(rusqlite::params_from_iter(pv), |row| {
        let valid_to: Option<String> = row.get(9)?;
        let stale = valid_to
            .as_deref()
            .map(|vt| vt < now_str.as_str())
            .unwrap_or(false);
        Ok(TemporalFact {
            id: row.get(0)?,
            project_id: row.get(1)?,
            node_id: row.get(2)?,
            fact_type: row.get(3)?,
            content: row.get(4)?,
            topic: row.get(5)?,
            tags: row.get(6)?,
            confidence: row.get(7)?,
            valid_from: row.get(8)?,
            valid_to,
            superseded_by: row.get(10)?,
            source_reference: row.get(11)?,
            provenance: row.get(12)?,
            repo_id: row.get(13)?,
            agent_id: row.get(14)?,
            stale,
            delegated: false,
        })
    })?;
    rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
}

pub(crate) fn read_facts(
    conn: &Connection,
    sql: &str,
    p: impl rusqlite::Params,
) -> Result<Vec<TemporalFact>> {
    let now_str = Utc::now().to_rfc3339();
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(p, |row| {
        let valid_to: Option<String> = row.get(9)?;
        let stale = valid_to
            .as_deref()
            .map(|vt| vt < now_str.as_str())
            .unwrap_or(false);
        Ok(TemporalFact {
            id: row.get(0)?,
            project_id: row.get(1)?,
            node_id: row.get(2)?,
            fact_type: row.get(3)?,
            content: row.get(4)?,
            topic: row.get(5)?,
            tags: row.get(6)?,
            confidence: row.get(7)?,
            valid_from: row.get(8)?,
            valid_to,
            superseded_by: row.get(10)?,
            source_reference: row.get(11)?,
            provenance: row.get(12)?,
            repo_id: row.get(13)?,
            agent_id: row.get(14)?,
            stale,
            delegated: false,
        })
    })?;
    rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
}

/// Parse an ISO 8601 duration (e.g. `P7D`, `PT2H`) and add it to `valid_from`.
pub(crate) fn compute_valid_to(valid_from: &str, ttl_iso: &str) -> Option<String> {
    let base: DateTime<Utc> = valid_from.parse().ok()?;
    let secs = parse_iso8601_duration_secs(ttl_iso)?;
    let end = base + chrono::Duration::seconds(secs);
    Some(end.to_rfc3339())
}

/// Minimal ISO 8601 duration parser supporting P[n]Y[n]M[n]DT[n]H[n]M[n]S.
fn parse_iso8601_duration_secs(s: &str) -> Option<i64> {
    let s = s.strip_prefix('P')?;
    let (date_part, time_part) = if let Some(idx) = s.find('T') {
        (&s[..idx], Some(&s[idx + 1..]))
    } else {
        (s, None)
    };
    let mut total: i64 = 0;
    total += parse_duration_segment(date_part, 'Y', 365 * 86400);
    total += parse_duration_segment(date_part, 'M', 30 * 86400);
    total += parse_duration_segment(date_part, 'W', 7 * 86400);
    total += parse_duration_segment(date_part, 'D', 86400);
    if let Some(tp) = time_part {
        total += parse_duration_segment(tp, 'H', 3600);
        total += parse_duration_segment(tp, 'M', 60);
        total += parse_duration_segment(tp, 'S', 1);
    }
    if total > 0 { Some(total) } else { None }
}

fn parse_duration_segment(s: &str, marker: char, multiplier: i64) -> i64 {
    if let Some(idx) = s.find(marker) {
        let num_start = s[..idx]
            .rfind(|c: char| !c.is_ascii_digit())
            .map(|i| i + 1)
            .unwrap_or(0);
        s[num_start..idx].parse::<i64>().unwrap_or(0) * multiplier
    } else {
        0
    }
}
