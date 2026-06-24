use anyhow::Result;
use chrono::Utc;
use rusqlite::Connection;

use crate::temporal_types::{FactType, TemporalFact};

pub(crate) fn query_facts_by_filter(
    conn: &Connection,
    project_id: &str,
    filter: &crate::temporal_types::FactFilter,
) -> Result<Vec<TemporalFact>> {
    let now = Utc::now().to_rfc3339();
    let mut sql = String::from(
        "SELECT id, project_id, node_id, fact_type, content, topic, tags,
                confidence, valid_from, valid_to, superseded_by,
                source_reference, provenance, repo_id, agent_id
         FROM temporal_facts WHERE project_id = ?1"
    );
    let mut param_vals: Vec<rusqlite::types::Value> = vec![project_id.to_string().into()];

    if !filter.include_expired {
        sql.push_str(" AND (valid_to IS NULL OR valid_to > ?2)");
        param_vals.push(now.clone().into());
        if let Some(ref ft) = filter.fact_type {
            sql.push_str(" AND fact_type = ?3");
            param_vals.push(ft.as_str().to_string().into());
        }
        if let Some(ref topic) = filter.topic {
            sql.push_str(" AND topic = ?4");
            param_vals.push(topic.clone().into());
        }
    } else {
        if let Some(ref ft) = filter.fact_type {
            sql.push_str(" AND fact_type = ?2");
            param_vals.push(ft.as_str().to_string().into());
        }
        if let Some(ref topic) = filter.topic {
            sql.push_str(" AND topic = ?3");
            param_vals.push(topic.clone().into());
        }
    }

    sql.push_str(" ORDER BY valid_from DESC");
    if let Some(lim) = filter.limit {
        sql.push_str(&format!(" LIMIT {lim}"));
    }

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(param_vals), |row| {
        let valid_to: Option<String> = row.get(9)?;
        let stale = valid_to
            .as_deref()
            .map(|v| v < now.as_str())
            .unwrap_or(false);
        Ok(TemporalFact {
            id: row.get(0)?,
            project_id: row.get(1)?,
            node_id: row.get(2)?,
            fact_type: FactType::parse_str(&row.get::<_, String>(3)?),
            content: row.get(4)?,
            topic: row.get(5)?,
            tags: serde_json::from_str(&row.get::<_, String>(6)?).unwrap_or_default(),
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

pub fn parse_ttl_to_rfc3339(ttl: &str) -> Option<String> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let s = ttl.trim().to_uppercase();
    if !s.starts_with('P') {
        return None;
    }
    let mut secs: u64 = 0;
    let body = &s[1..];
    let (date_part, time_part) = if let Some(t) = body.find('T') {
        (&body[..t], &body[t + 1..])
    } else {
        (body, "")
    };
    secs += parse_duration_part(date_part, &[('D', 86400), ('W', 604800)]);
    secs += parse_duration_part(time_part, &[('H', 3600), ('M', 60), ('S', 1)]);
    if secs == 0 {
        return None;
    }
    let expiry = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() + secs)?;
    let dt = chrono::DateTime::<chrono::Utc>::from_timestamp(expiry as i64, 0)?;
    Some(dt.to_rfc3339())
}

fn parse_duration_part(s: &str, units: &[(char, u64)]) -> u64 {
    let mut total = 0u64;
    let mut num_buf = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() {
            num_buf.push(c);
        } else if let Some(&(_, mul)) = units.iter().find(|(u, _)| *u == c) {
            let n: u64 = num_buf.parse().unwrap_or(0);
            total += n * mul;
            num_buf.clear();
        }
    }
    total
}
