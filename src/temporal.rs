pub use crate::temporal_types::{AddFactInput, FactFilter, FactType, TemporalFact};

use crate::temporal_query::{parse_ttl_to_rfc3339, query_facts_by_filter};
use anyhow::Result;
use rusqlite::{params, Connection};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

pub struct TemporalStore {
    db: TempConn,
    project_id: String,
}

enum TempConn {
    Shared(Arc<Mutex<Connection>>),
    Borrowed(*const Connection),
}

unsafe impl Send for TempConn {}
unsafe impl Sync for TempConn {}

impl TemporalStore {
    pub fn new(db: Arc<Mutex<Connection>>, project_id: &str) -> Self {
        Self {
            db: TempConn::Shared(db),
            project_id: project_id.to_string(),
        }
    }

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
                let conn = unsafe { &**ptr };
                f(conn)
            }
        }
    }

    pub fn add_fact(&self, input: AddFactInput<'_>) -> Result<String> {
        self.with_conn(|conn| {
            let id = Uuid::new_v4().to_string();
            let now = chrono::Utc::now().to_rfc3339();
            let tags_json = serde_json::to_string(&input.tags).unwrap_or_else(|_| "[]".to_string());
            let valid_to = input.ttl.and_then(|ttl| parse_ttl_to_rfc3339(ttl));

            conn.execute(
                "INSERT INTO temporal_facts
                 (id, project_id, node_id, fact_type, content, topic, tags, confidence,
                  valid_from, valid_to, source_reference, provenance, repo_id, agent_id)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
                params!(id, self.project_id, input.node_id, input.fact_type.as_str(),
                    input.content, input.topic, tags_json, input.confidence,
                    now, valid_to, input.source_reference, input.provenance,
                    input.repo_id, input.agent_id),
            )?;
            Ok(id)
        })
    }

    pub fn expire_fact(&self, fact_id: &str, superseded_by: Option<&str>) -> Result<()> {
        self.with_conn(|conn| {
            let now = chrono::Utc::now().to_rfc3339();
            conn.execute(
                "UPDATE temporal_facts SET valid_to = ?1, superseded_by = ?2
                 WHERE id = ?3 AND project_id = ?4",
                rusqlite::params![now, superseded_by, fact_id, self.project_id],
            )?;
            Ok(())
        })
    }

    pub fn invalidate_fact(&self, fact_id: &str, superseded_by: Option<&str>) -> Result<()> {
        self.expire_fact(fact_id, superseded_by)
    }

    pub fn get_active_facts(&self, filter: &FactFilter) -> Result<Vec<TemporalFact>> {
        self.with_conn(|conn| query_facts_by_filter(conn, &self.project_id, filter))
    }

    pub fn get_facts_for_node(&self, node_id: &str) -> Result<Vec<TemporalFact>> {
        let filter = FactFilter {
            node_id: Some(node_id.to_string()),
            ..Default::default()
        };
        self.get_active_facts(&filter)
    }

    pub fn get_fact_history(&self, node_id: &str) -> Result<Vec<TemporalFact>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, project_id, node_id, fact_type, content, topic, tags, confidence,
                        valid_from, valid_to, superseded_by, source_reference, provenance, repo_id, agent_id
                 FROM temporal_facts
                 WHERE project_id = ?1 AND node_id = ?2
                 ORDER BY valid_from DESC",
            )?;
            let rows = stmt
                .query_map(params![self.project_id, node_id], |row| {
                    let tags_str: Option<String> = row.get(6)?;
                    let tags: Vec<String> = tags_str
                        .and_then(|s| serde_json::from_str(&s).ok())
                        .unwrap_or_default();
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
                        valid_to: row.get(9)?,
                        superseded_by: row.get(10)?,
                        source_reference: row.get(11)?,
                        provenance: row.get(12)?,
                        repo_id: row.get(13)?,
                        agent_id: row.get(14)?,
                        stale: false,
                        delegated: false,
                    })
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }
}

impl Default for AddFactInput<'_> {
    fn default() -> Self {
        AddFactInput {
            node_id: None,
            fact_type: FactType::Decision,
            content: "",
            topic: None,
            tags: vec![],
            confidence: None,
            ttl: None,
            source_reference: None,
            provenance: None,
            repo_id: None,
            agent_id: None,
        }
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

        let input = AddFactInput {
            node_id: None,
            fact_type: FactType::Architecture,
            content: "Backend uses Axum + Tokio",
            topic: None,
            tags: vec![],
            confidence: None,
            ttl: None,
            source_reference: Some("initial setup"),
            provenance: None,
            repo_id: None,
            agent_id: None,
        };
        let id = store.add_fact(input).unwrap();

        let facts = store.get_active_facts(&FactFilter::default()).unwrap();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].id, id);
        assert_eq!(facts[0].content, "Backend uses Axum + Tokio");
        assert!(facts[0].valid_to.is_none());
    }

    #[test]
    fn expire_fact_sets_valid_to() {
        let engine = HermesEngine::in_memory("test").unwrap();
        let store = TemporalStore::new(engine.db().clone(), "test");

        let id = store.add_fact(AddFactInput {
            node_id: None,
            fact_type: FactType::Decision,
            content: "Use SQLite for storage",
            ..Default::default()
        }).unwrap();

        store.expire_fact(&id, None).unwrap();
        let active = store.get_active_facts(&FactFilter::default()).unwrap();
        assert!(active.is_empty());
    }

    #[test]
    fn filter_by_fact_type() {
        let engine = HermesEngine::in_memory("test").unwrap();
        let store = TemporalStore::new(engine.db().clone(), "test");

        store.add_fact(AddFactInput {
            fact_type: FactType::Architecture,
            content: "Axum backend",
            ..Default::default()
        }).unwrap();
        store.add_fact(AddFactInput {
            fact_type: FactType::Decision,
            content: "Use Rust",
            ..Default::default()
        }).unwrap();

        let arch_facts = store.get_active_facts(&FactFilter {
            fact_type: Some(FactType::Architecture),
            ..Default::default()
        }).unwrap();
        assert_eq!(arch_facts.len(), 1);
        assert_eq!(arch_facts[0].content, "Axum backend");
    }

    #[test]
    fn fact_type_parse_str_unknown_falls_back_to_decision() {
        assert_eq!(FactType::parse_str("unknown_type"), FactType::Decision);
    }

    #[test]
    fn fact_type_roundtrip() {
        let variants = [
            FactType::Architecture,
            FactType::ApiContract,
            FactType::Decision,
            FactType::ErrorPattern,
            FactType::Constraint,
            FactType::Learning,
            FactType::Assumption,
            FactType::Observation,
            FactType::Dependency,
        ];
        for v in &variants {
            assert_eq!(&FactType::parse_str(v.as_str()), v);
        }
    }
}
