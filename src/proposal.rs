// tools/hermes-engine/src/proposal.rs
//
// Proposal lifecycle — store, query, approve (creates mission).

use anyhow::Result;
use chrono::Utc;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::mission::MissionStore;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proposal {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub description: Option<String>,
    pub source: String,
    pub priority: i64,
    pub status: String,
    pub evidence_ids: Option<String>,
    pub why_it_matters: Option<String>,
    pub next_step: Option<String>,
    pub rejected_reason: Option<String>,
    pub mission_id: Option<String>,
    pub repo: Option<String>,
    pub fingerprint: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

pub struct ProposalStore<'a> {
    conn: &'a Connection,
    project_id: &'a str,
}

impl<'a> ProposalStore<'a> {
    pub fn new(conn: &'a Connection, project_id: &'a str) -> Self {
        Self { conn, project_id }
    }

    pub fn create_batch(&self, proposals: &[Value]) -> Result<Vec<Proposal>> {
        let now = Utc::now().to_rfc3339();
        let mut created = Vec::new();
        for p in proposals {
            let id = p["id"].as_str().map(String::from).unwrap_or_else(|| Uuid::new_v4().to_string());
            let title = p["title"].as_str().unwrap_or("");
            anyhow::ensure!(!title.is_empty(), "proposal requires 'title'");
            let source = p["source"].as_str().unwrap_or("ideation");
            let priority = p["priority"].as_i64().unwrap_or(5);
            let status = p["status"].as_str().unwrap_or("pending");
            let description = p["description"].as_str();
            let evidence_ids = p["evidence_ids"].as_array().map(|a| serde_json::to_string(a).unwrap_or_default());
            let why_it_matters = p["why_it_matters"].as_str();
            let next_step = p["next_step"].as_str();
            let repo = p["repo"].as_str();
            let fingerprint = p["fingerprint"].as_str();

            self.conn.execute(
                "INSERT INTO proposals (id, project_id, title, description, source, priority, status,
                        evidence_ids, why_it_matters, next_step, repo, fingerprint, created_at, updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)
                 ON CONFLICT(id) DO UPDATE SET
                     title=excluded.title, description=excluded.description,
                     source=excluded.source, priority=excluded.priority,
                     evidence_ids=excluded.evidence_ids,
                     why_it_matters=excluded.why_it_matters,
                     next_step=excluded.next_step, repo=excluded.repo,
                     fingerprint=excluded.fingerprint, updated_at=excluded.updated_at",
                params![
                    id, self.project_id, title, description, source, priority, status,
                    evidence_ids, why_it_matters, next_step, repo, fingerprint, now, now
                ],
            )?;
            created.push(self.get(&id)?);
        }
        Ok(created)
    }

    pub fn get(&self, proposal_id: &str) -> Result<Proposal> {
        self.conn
            .query_row(
                "SELECT id, project_id, title, description, source, priority, status,
                        evidence_ids, why_it_matters, next_step, rejected_reason,
                        mission_id, repo, fingerprint, created_at, updated_at
                 FROM proposals WHERE id = ?1",
                params![proposal_id],
                |row| {
                    Ok(Proposal {
                        id: row.get(0)?,
                        project_id: row.get(1)?,
                        title: row.get(2)?,
                        description: row.get(3)?,
                        source: row.get(4)?,
                        priority: row.get(5)?,
                        status: row.get(6)?,
                        evidence_ids: row.get(7)?,
                        why_it_matters: row.get(8)?,
                        next_step: row.get(9)?,
                        rejected_reason: row.get(10)?,
                        mission_id: row.get(11)?,
                        repo: row.get(12)?,
                        fingerprint: row.get(13)?,
                        created_at: row.get(14)?,
                        updated_at: row.get(15)?,
                    })
                },
            )
            .map_err(|e| anyhow::anyhow!("proposal not found: {e}"))
    }

    pub fn list(&self, status: Option<&str>, source: Option<&str>, limit: usize) -> Result<Vec<Proposal>> {
        let mut sql = String::from(
            "SELECT id, project_id, title, description, source, priority, status,
                    evidence_ids, why_it_matters, next_step, rejected_reason,
                    mission_id, repo, fingerprint, created_at, updated_at
             FROM proposals WHERE project_id = ?1"
        );
        let mut pv: Vec<rusqlite::types::Value> = vec![rusqlite::types::Value::from(self.project_id.to_string())];

        if let Some(st) = status {
            pv.push(rusqlite::types::Value::from(st.to_string()));
            sql.push_str(&format!(" AND status = ?{}", pv.len()));
        }
        if let Some(src) = source {
            pv.push(rusqlite::types::Value::from(src.to_string()));
            sql.push_str(&format!(" AND source = ?{}", pv.len()));
        }
        pv.push(rusqlite::types::Value::from(limit as i64));
        sql.push_str(&format!(" ORDER BY updated_at DESC LIMIT ?{}", pv.len()));

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(pv), |row| {
            Ok(Proposal {
                id: row.get(0)?,
                project_id: row.get(1)?,
                title: row.get(2)?,
                description: row.get(3)?,
                source: row.get(4)?,
                priority: row.get(5)?,
                status: row.get(6)?,
                evidence_ids: row.get(7)?,
                why_it_matters: row.get(8)?,
                next_step: row.get(9)?,
                rejected_reason: row.get(10)?,
                mission_id: row.get(11)?,
                repo: row.get(12)?,
                fingerprint: row.get(13)?,
                created_at: row.get(14)?,
                updated_at: row.get(15)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn update(&self, proposal_id: &str, args: &Value) -> Result<Proposal> {
        let now = Utc::now().to_rfc3339();
        let priority_str = args["priority"].as_i64().map(|v| v.to_string());
        let sets: Vec<(&str, &str)> = [
            ("title", args["title"].as_str()),
            ("description", args["description"].as_str()),
            ("priority", priority_str.as_deref()),
            ("next_step", args["next_step"].as_str()),
            ("why_it_matters", args["why_it_matters"].as_str()),
        ]
        .into_iter()
        .filter_map(|(col, v)| v.map(|val| (col, val)))
        .collect();

        for (col, val) in &sets {
            let sql = format!("UPDATE proposals SET {col} = ?1, updated_at = ?2 WHERE id = ?3");
            self.conn.execute(&sql, params![val, now, proposal_id])?;
        }
        self.get(proposal_id)
    }

    pub fn reject(&self, proposal_id: &str, reason: &str) -> Result<Proposal> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE proposals SET status = 'rejected', rejected_reason = ?1, updated_at = ?2 WHERE id = ?3",
            params![reason, now, proposal_id],
        )?;
        self.get(proposal_id)
    }

    /// Generic status update with transition validation.
    ///
    /// Allowed transitions:
    /// - `pending` -> `edited` | `approved` | `rejected`
    /// - `edited` -> `approved` | `rejected` | `pending`
    /// - `approved` -> `completed`
    /// - `rejected` -> terminal
    /// - `completed` -> terminal
    pub fn update_status(&self, proposal_id: &str, new_status: &str) -> Result<Proposal> {
        let current = self.get(proposal_id)?;
        if !is_valid_proposal_status_transition(&current.status, new_status) {
            anyhow::bail!(
                "invalid proposal status transition: {} -> {}",
                current.status,
                new_status
            );
        }
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE proposals SET status = ?1, updated_at = ?2 WHERE id = ?3",
            params![new_status, now, proposal_id],
        )?;
        self.get(proposal_id)
    }

    pub fn approve(&self, _engine: &crate::HermesEngine, conn: &Connection, proposal_id: &str) -> Result<Value> {
        let proposal = self.get(proposal_id)?;
        anyhow::ensure!(
            proposal.status == "pending" || proposal.status == "edited",
            "only pending or edited proposals can be approved, current: {}",
            proposal.status
        );

        let mission_store = MissionStore::new(conn, self.project_id);
        let title = format!("Proposal: {}", proposal.title);
        let description = proposal.description.clone().or(proposal.next_step.clone());
        let tags = Some(format!("proposal,source:{}", proposal.source));
        let mission = mission_store.create(&title, description.as_deref(), tags.as_deref())?;

        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE proposals SET status = 'approved', mission_id = ?1, updated_at = ?2 WHERE id = ?3",
            params![mission.id, now, proposal_id],
        )?;

        let updated = self.get(proposal_id)?;
        Ok(json!({
            "proposal": updated,
            "mission": {
                "mission_id": mission.id,
                "status": mission.status,
                "created_at": mission.created_at,
            }
        }))
    }
}

fn is_valid_proposal_status_transition(from: &str, to: &str) -> bool {
    matches!(
        (from, to),
        ("pending", "edited")
            | ("pending", "approved")
            | ("pending", "rejected")
            | ("edited", "approved")
            | ("edited", "rejected")
            | ("edited", "pending")
            | ("approved", "completed")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transition_matrix() {
        // Valid transitions
        assert!(is_valid_proposal_status_transition("pending", "edited"));
        assert!(is_valid_proposal_status_transition("pending", "approved"));
        assert!(is_valid_proposal_status_transition("pending", "rejected"));
        assert!(is_valid_proposal_status_transition("edited", "approved"));
        assert!(is_valid_proposal_status_transition("edited", "rejected"));
        assert!(is_valid_proposal_status_transition("edited", "pending"));
        assert!(is_valid_proposal_status_transition("approved", "completed"));

        // Invalid transitions
        assert!(!is_valid_proposal_status_transition("edited", "completed"));
        assert!(!is_valid_proposal_status_transition("pending", "completed"));
        assert!(!is_valid_proposal_status_transition("rejected", "approved"));
        assert!(!is_valid_proposal_status_transition("completed", "pending"));
        assert!(!is_valid_proposal_status_transition("approved", "rejected"));
    }
}
