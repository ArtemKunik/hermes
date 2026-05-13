use anyhow::Result;
use chrono::Utc;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use crate::lock_ext::LockExt;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalFact {
    pub id: String,
    pub project_id: String,
    pub node_id: Option<String>,
    pub fact_type: FactType,
    pub content: String,
    pub valid_from: String,
    pub valid_to: Option<String>,
    pub superseded_by: Option<String>,
    pub source_reference: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum FactType {
    Architecture,
    ApiContract,
    Decision,
    ErrorPattern,
    Constraint,
    Learning,
}

impl FactType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Architecture => "architecture",
            Self::ApiContract => "api_contract",
            Self::Decision => "decision",
            Self::ErrorPattern => "error_pattern",
            Self::Constraint => "constraint",
            Self::Learning => "learning",
        }
    }

    pub fn parse_str(s: &str) -> Self {
        match s {
            "architecture" => Self::Architecture,
            "api_contract" => Self::ApiContract,
            "decision" => Self::Decision,
            "error_pattern" => Self::ErrorPattern,
            "constraint" => Self::Constraint,
            "learning" => Self::Learning,
            _ => Self::Decision,
        }
    }
}

pub struct TemporalStore {
    db: Arc<Mutex<Connection>>,
    project_id: String,
}

impl TemporalStore {
    pub fn new(db: Arc<Mutex<Connection>>, project_id: &str) -> Self {
        Self {
            db,
            project_id: project_id.to_string(),
        }
    }

    pub fn add_fact(
        &self,
        node_id: Option<&str>,
        fact_type: FactType,
        content: &str,
        source_reference: Option<&str>,
    ) -> Result<String> {
        let conn = self.db.lock_ctx("temporal")?;
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();

        conn.execute(
            "INSERT INTO temporal_facts
             (id, project_id, node_id, fact_type, content, valid_from, source_reference)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                id,
                self.project_id,
                node_id,
                fact_type.as_str(),
                content,
                now,
                source_reference,
            ],
        )?;
        Ok(id)
    }

    pub fn invalidate_fact(&self, fact_id: &str, superseded_by: Option<&str>) -> Result<()> {
        let conn = self.db.lock_ctx("temporal")?;
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE temporal_facts SET valid_to = ?1, superseded_by = ?2
             WHERE id = ?3 AND project_id = ?4",
            params![now, superseded_by, fact_id, self.project_id],
        )?;
        Ok(())
    }

    pub fn get_active_facts(&self, fact_type: Option<&FactType>) -> Result<Vec<TemporalFact>> {
        let conn = self.db.lock_ctx("temporal")?;

        let (sql, fact_type_str);
        let base_params: Vec<&dyn rusqlite::types::ToSql>;

        if let Some(ft) = fact_type {
            sql = "SELECT id, project_id, node_id, fact_type, content, valid_from, valid_to, superseded_by, source_reference
                   FROM temporal_facts
                   WHERE project_id = ?1 AND valid_to IS NULL AND fact_type = ?2
                   ORDER BY valid_from DESC";
            fact_type_str = ft.as_str().to_string();
            base_params = vec![
                &self.project_id as &dyn rusqlite::types::ToSql,
                &fact_type_str,
            ];
        } else {
            sql = "SELECT id, project_id, node_id, fact_type, content, valid_from, valid_to, superseded_by, source_reference
                   FROM temporal_facts
                   WHERE project_id = ?1 AND valid_to IS NULL
                   ORDER BY valid_from DESC";
            base_params = vec![&self.project_id as &dyn rusqlite::types::ToSql];
        }

        let mut stmt = conn.prepare(sql)?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(base_params), Self::map_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn get_fact_history(&self, node_id: &str) -> Result<Vec<TemporalFact>> {
        let conn = self.db.lock_ctx("temporal")?;
        let mut stmt = conn.prepare(
            "SELECT id, project_id, node_id, fact_type, content, valid_from, valid_to, superseded_by, source_reference
             FROM temporal_facts
             WHERE project_id = ?1 AND node_id = ?2
             ORDER BY valid_from DESC",
        )?;
        let rows = stmt
            .query_map(params![self.project_id, node_id], Self::map_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn map_row(row: &rusqlite::Row) -> rusqlite::Result<TemporalFact> {
        Ok(TemporalFact {
            id: row.get(0)?,
            project_id: row.get(1)?,
            node_id: row.get(2)?,
            fact_type: FactType::parse_str(&row.get::<_, String>(3)?),
            content: row.get(4)?,
            valid_from: row.get(5)?,
            valid_to: row.get(6)?,
            superseded_by: row.get(7)?,
            source_reference: row.get(8)?,
        })
    }
}

#[cfg(test)]
#[path = "temporal_tests.rs"]
mod tests;
