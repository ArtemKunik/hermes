// ChartApp/hermes-engine/src/temporal.rs
use anyhow::Result;
use chrono::Utc;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

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
        let conn = self.db.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
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
        let conn = self.db.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE temporal_facts SET valid_to = ?1, superseded_by = ?2
             WHERE id = ?3 AND project_id = ?4",
            params![now, superseded_by, fact_id, self.project_id],
        )?;
        Ok(())
    }

    pub fn get_active_facts(&self, fact_type: Option<&FactType>) -> Result<Vec<TemporalFact>> {
        let conn = self.db.lock().map_err(|e| anyhow::anyhow!("{e}"))?;

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
        let conn = self.db.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
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
mod tests {
    use super::*;
    use crate::HermesEngine;

    #[test]
    fn add_and_retrieve_fact() {
        let engine = HermesEngine::in_memory("test").unwrap();
        let store = TemporalStore::new(engine.db().clone(), "test");

        let id = store
            .add_fact(
                None,
                FactType::Architecture,
                "Backend uses Axum + Tokio",
                Some("initial setup"),
            )
            .unwrap();

        let facts = store.get_active_facts(None).unwrap();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].id, id);
        assert_eq!(facts[0].content, "Backend uses Axum + Tokio");
        assert!(facts[0].valid_to.is_none());
    }

    #[test]
    fn invalidate_fact_sets_valid_to() {
        let engine = HermesEngine::in_memory("test").unwrap();
        let store = TemporalStore::new(engine.db().clone(), "test");

        let id = store
            .add_fact(None, FactType::Decision, "Use SQLite for storage", None)
            .unwrap();

        store.invalidate_fact(&id, None).unwrap();

        let active = store.get_active_facts(None).unwrap();
        assert!(active.is_empty());
    }

    #[test]
    fn supersede_fact_creates_chain() {
        let engine = HermesEngine::in_memory("test").unwrap();
        let store = TemporalStore::new(engine.db().clone(), "test");

        let old_id = store
            .add_fact(None, FactType::Decision, "Use ChromaDB", None)
            .unwrap();

        let new_id = store
            .add_fact(None, FactType::Decision, "Use Qdrant instead", None)
            .unwrap();

        store.invalidate_fact(&old_id, Some(&new_id)).unwrap();

        let active = store.get_active_facts(None).unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].content, "Use Qdrant instead");
    }

    #[test]
    fn filter_by_fact_type() {
        let engine = HermesEngine::in_memory("test").unwrap();
        let store = TemporalStore::new(engine.db().clone(), "test");

        store
            .add_fact(None, FactType::Architecture, "Axum backend", None)
            .unwrap();
        store
            .add_fact(None, FactType::Decision, "Use Rust", None)
            .unwrap();

        let arch_facts = store
            .get_active_facts(Some(&FactType::Architecture))
            .unwrap();
        assert_eq!(arch_facts.len(), 1);
        assert_eq!(arch_facts[0].content, "Axum backend");
    }
}
