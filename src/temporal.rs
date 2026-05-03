// ChartApp/hermes-engine/src/temporal.rs
use anyhow::Result;
use chrono::Utc;
use rusqlite::{params, Connection};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

pub use crate::temporal_types::{AddFactInput, FactFilter, FactType, TemporalFact};

pub struct TemporalStore {
    db: TempConn,
    project_id: String,
}

enum TempConn {
    Shared(Arc<Mutex<Connection>>),
    Borrowed(*const Connection),
}

// Safety: TemporalStore is used in threads, but we ensure the lifetime of
// the borrowed connection exceeds the graph during execute_tool_call.
unsafe impl Send for TempConn {}
unsafe impl Sync for TempConn {}

impl TemporalStore {
    pub fn new(db: Arc<Mutex<Connection>>, project_id: &str) -> Self {
        Self {
            db: TempConn::Shared(db),
            project_id: project_id.to_string(),
        }
    }

    /// TRACK-066: Create a temporal store from a raw connection (read-only isolation).
    pub fn from_conn(conn: &Connection, project_id: &str) -> Self {
        Self {
            db: TempConn::Borrowed(conn as *const Connection),
            project_id: project_id.to_string(),
        }
    }

    fn with_conn<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&Connection) -> Result<R>,
    {
        match &self.db {
            TempConn::Shared(arc) => {
                let conn = arc.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
                f(&conn)
            }
            TempConn::Borrowed(ptr) => {
                // Safety: ptr is valid for the duration of the tool call.
                let conn = unsafe { &**ptr };
                f(conn)
            }
        }
    }

    pub fn add_fact(&self, input: AddFactInput<'_>) -> Result<String> {
        self.with_conn(|conn| {
            let id = Uuid::new_v4().to_string();
            let now = Utc::now().to_rfc3339();
            let tags_json = serde_json::to_string(&input.tags).unwrap_or_else(|_| "[]".to_string());
            let valid_to = input.ttl.and_then(|ttl| parse_ttl_to_rfc3339(ttl, &now));

            conn.execute(
                "INSERT INTO temporal_facts
                 (id, project_id, node_id, fact_type, content, topic, tags, confidence,
                  valid_from, valid_to, source_reference, provenance, repo_id, agent_id)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
                params![
                    id,
                    self.project_id,
                    input.node_id,
                    input.fact_type.as_str(),
                    input.content,
                    input.topic,
                    tags_json,
                    input.confidence,
                    now,
                    valid_to,
                    input.source_reference,
                    input.provenance,
                    input.repo_id,
                    input.agent_id,
                ],
            )?;
            Ok(id)
        })
    }

    pub fn expire_fact(&self, fact_id: &str, superseded_by: Option<&str>) -> Result<()> {
        self.with_conn(|conn| {
            let now = Utc::now().to_rfc3339();
            conn.execute(
                "UPDATE temporal_facts SET valid_to = ?1, superseded_by = ?2
                 WHERE id = ?3 AND project_id = ?4",
                params![now, superseded_by, fact_id, self.project_id],
            )?;
            Ok(())
        })
    }

    /// Legacy alias kept for callers not yet migrated.
    pub fn invalidate_fact(&self, fact_id: &str, superseded_by: Option<&str>) -> Result<()> {
        self.expire_fact(fact_id, superseded_by)
    }

    pub fn get_active_facts(&self, filter: &FactFilter) -> Result<Vec<TemporalFact>> {
        self.with_conn(|conn| {
            let now = Utc::now().to_rfc3339();
            let mut sql = "SELECT id, project_id, node_id, fact_type, content, topic, tags,
                                  confidence, valid_from, valid_to, superseded_by,
                                  source_reference, provenance, repo_id, agent_id
                           FROM temporal_facts WHERE project_id = ?1".to_string();
            let mut param_vals: Vec<rusqlite::types::Value> = vec![self.project_id.clone().into()];

            if !filter.include_expired {
                sql.push_str(" AND (valid_to IS NULL OR valid_to > ?2)");
                param_vals.push(now.clone().into());
                if let Some(ft) = &filter.fact_type {
                    sql.push_str(" AND fact_type = ?3");
                    param_vals.push(ft.as_str().to_string().into());
                }
                if let Some(topic) = &filter.topic {
                    let idx = param_vals.len() + 1;
                    sql.push_str(&format!(" AND topic = ?{idx}"));
                    param_vals.push(topic.clone().into());
                }
                if let Some(repo) = &filter.repo_id {
                    let idx = param_vals.len() + 1;
                    sql.push_str(&format!(" AND repo_id = ?{idx}"));
                    param_vals.push(repo.clone().into());
                }
            } else {
                if let Some(ft) = &filter.fact_type {
                    sql.push_str(" AND fact_type = ?2");
                    param_vals.push(ft.as_str().to_string().into());
                }
                if let Some(topic) = &filter.topic {
                    let idx = param_vals.len() + 1;
                    sql.push_str(&format!(" AND topic = ?{idx}"));
                    param_vals.push(topic.clone().into());
                }
                if let Some(repo) = &filter.repo_id {
                    let idx = param_vals.len() + 1;
                    sql.push_str(&format!(" AND repo_id = ?{idx}"));
                    param_vals.push(repo.clone().into());
                }
            }

            sql.push_str(" ORDER BY valid_from DESC");
            if let Some(lim) = filter.limit {
                sql.push_str(&format!(" LIMIT {lim}"));
            }

            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(param_vals), |row| {
                let tags_raw: Option<String> = row.get(6)?;
                let tags: Vec<String> = tags_raw
                    .as_deref()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or_default();
                let valid_to: Option<String> = row.get(9)?;
                let stale = valid_to.as_deref().map(|v| v < now.as_str()).unwrap_or(false);
                Ok(TemporalFact {
                    id: row.get(0)?,
                    project_id: row.get(1)?,
                    node_id: row.get(2)?,
                    fact_type: FactType::parse_str(&row.get::<_, String>(3)?),
                    content: row.get(4)?,
                    topic: row.get(5)?,
                    tags,
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

            let tag_filter = filter.tags.as_deref().unwrap_or(&[]);
            let mut results = Vec::new();
            for row in rows {
                let fact = row?;
                if !tag_filter.is_empty() && !tag_filter.iter().all(|t| fact.tags.contains(t)) {
                    continue;
                }
                results.push(fact);
            }
            Ok(results)
        })
    }

    pub fn get_facts_for_node(&self, node_id: &str) -> Result<Vec<TemporalFact>> {
        let filter = FactFilter { node_id: Some(node_id.to_string()), ..Default::default() };
        self.get_active_facts(&filter)
    }
}

/// Parse ISO 8601 duration string (e.g. "P7D", "PT1H") into an RFC 3339 expiry timestamp.
fn parse_ttl_to_rfc3339(ttl: &str, _now_rfc3339: &str) -> Option<String> {
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
    let dt = chrono::DateTime::<Utc>::from_timestamp(expiry as i64, 0)?;
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
