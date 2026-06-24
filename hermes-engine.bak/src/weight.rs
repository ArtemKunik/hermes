// tools/hermes-engine/src/weight.rs
//
// AD-02: Weight index — per-node reinforcement/decay tracking.
// Scores from FTS/vector search are multiplied by weight before final ranking.
//
// Invariant: weight is clamped to [0.0, 2.0] on every write.

use anyhow::Result;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

const WEIGHT_MIN: f64 = 0.0;
const WEIGHT_MAX: f64 = 2.0;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeightRecord {
    pub node_id: String,
    pub weight: f64,
    pub reinforcement_count: i64,
    pub decay_count: i64,
    pub last_updated: String,
}

pub struct WeightStore {
    db: WeightConn,
}

enum WeightConn {
    Shared(Arc<Mutex<Connection>>),
    Borrowed(*const Connection),
}

// Safety: ptr is valid for the duration of the tool call.
unsafe impl Send for WeightConn {}
unsafe impl Sync for WeightConn {}

impl WeightStore {
    pub fn new(db: Arc<Mutex<Connection>>) -> Self {
        WeightStore { db: WeightConn::Shared(db) }
    }

    /// TRACK-066: Create a weight store from a raw connection (read-only isolation).
    pub fn from_conn(conn: &Connection) -> Self {
        WeightStore { db: WeightConn::Borrowed(conn as *const Connection) }
    }

    fn with_conn<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&Connection) -> Result<R>,
    {
        match &self.db {
            WeightConn::Shared(arc) => {
                let conn = arc.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
                f(&conn)
            }
            WeightConn::Borrowed(ptr) => {
                // Safety: ptr is valid for the duration of the tool call.
                let conn = unsafe { &**ptr };
                f(conn)
            }
        }
    }

    /// Return the weight for a node, defaulting to 1.0 if absent.
    pub fn get_weight(&self, node_id: &str) -> Result<f64> {
        self.with_conn(|db| {
            let result: rusqlite::Result<f64> = db.query_row(
                "SELECT weight FROM weight_index WHERE node_id = ?1",
                params![node_id],
                |row| row.get(0),
            );
            match result {
                Ok(w) => Ok(w),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(1.0),
                Err(e) => Err(e.into()),
            }
        })
    }

    /// Get the full record for a node (None if no explicit entry exists).
    pub fn get_record(&self, node_id: &str) -> Result<Option<WeightRecord>> {
        self.with_conn(|db| {
            let mut stmt = db.prepare(
                "SELECT node_id, weight, reinforcement_count, decay_count, last_updated \
                 FROM weight_index WHERE node_id = ?1",
            )?;
            let record = stmt
                .query_row(params![node_id], |row| {
                    Ok(WeightRecord {
                        node_id: row.get(0)?,
                        weight: row.get(1)?,
                        reinforcement_count: row.get(2)?,
                        decay_count: row.get(3)?,
                        last_updated: row.get(4)?,
                    })
                })
                .optional()?;
            Ok(record)
        })
    }

    /// Batch-fetch weights for many nodes at once.
    pub fn get_weights_for(&self, node_ids: &[&str]) -> Result<std::collections::HashMap<String, f64>> {
        if node_ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        self.with_conn(|db| {
            let placeholders = vec!["?"; node_ids.len()].join(",");
            let sql = format!(
                "SELECT node_id, weight FROM weight_index WHERE node_id IN ({})",
                placeholders
            );
            let mut stmt = db.prepare(&sql)?;
            let mut rows = stmt.query(rusqlite::params_from_iter(node_ids))?;
            let mut map = std::collections::HashMap::new();
            while let Some(row) = rows.next()? {
                map.insert(row.get(0)?, row.get(1)?);
            }
            Ok(map)
        })
    }

    /// Adjust a node's weight by a delta (+ for reinforcement, - for decay).
    pub fn adjust_weight(&self, node_id: &str, delta: f64) -> Result<WeightRecord> {
        self.with_conn(|db| {
            let now = chrono::Utc::now().to_rfc3339();
            let current = self.get_record(node_id)?;

            let (new_w, new_r, new_d) = match current {
                Some(r) => {
                    let mut w = r.weight + delta;
                    w = w.clamp(WEIGHT_MIN, WEIGHT_MAX);
                    let mut rc = r.reinforcement_count;
                    let mut dc = r.decay_count;
                    if delta > 0.0 {
                        rc += 1;
                    } else {
                        dc += 1;
                    }
                    (w, rc, dc)
                }
                None => {
                    let mut w = 1.0 + delta;
                    w = w.clamp(WEIGHT_MIN, WEIGHT_MAX);
                    let (rc, dc) = if delta > 0.0 { (1, 0) } else { (0, 1) };
                    (w, rc, dc)
                }
            };

            db.execute(
                "INSERT OR REPLACE INTO weight_index \
                 (node_id, weight, reinforcement_count, decay_count, last_updated) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![node_id, new_w, new_r, new_d, now],
            )?;

            Ok(WeightRecord {
                node_id: node_id.to_string(),
                weight: new_w,
                reinforcement_count: new_r,
                decay_count: new_d,
                last_updated: now,
            })
        })
    }

    /// List all nodes with weights NOT equal to 1.0.
    pub fn list_non_default(&self) -> Result<Vec<WeightRecord>> {
        self.with_conn(|db| {
            let mut stmt = db.prepare(
                "SELECT node_id, weight, reinforcement_count, decay_count, last_updated \
                 FROM weight_index WHERE weight != 1.0 ORDER BY last_updated DESC",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(WeightRecord {
                    node_id: row.get(0)?,
                    weight: row.get(1)?,
                    reinforcement_count: row.get(2)?,
                    decay_count: row.get(3)?,
                    last_updated: row.get(4)?,
                })
            })?;
            let mut results = Vec::new();
            for r in rows {
                results.push(r?);
            }
            Ok(results)
        })
    }

    /// Returns ALL nodes in the graph for a project, merged with their weights.
    /// This is used for cross-session consolidation (AD-04).
    pub fn list_all_nodes_with_weights(&self, project_id: &str) -> Result<Vec<WeightRecord>> {
        self.with_conn(|db| {
            let mut stmt = db.prepare(
                "SELECT n.id, COALESCE(w.weight, 1.0), \
                        COALESCE(w.reinforcement_count, 0), COALESCE(w.decay_count, 0), \
                        COALESCE(w.last_updated, '') \
                 FROM nodes n \
                 LEFT JOIN weight_index w ON w.node_id = n.id \
                 WHERE n.project_id = ?1",
            )?;
            let rows = stmt.query_map(params![project_id], |row| {
                Ok(WeightRecord {
                    node_id: row.get(0)?,
                    weight: row.get(1)?,
                    reinforcement_count: row.get(2)?,
                    decay_count: row.get(3)?,
                    last_updated: row.get(4)?,
                })
            })?;
            let mut results = Vec::new();
            for r in rows {
                results.push(r?);
            }
            Ok(results)
        })
    }
}

trait OptionalRow {
    fn optional(self) -> Result<Option<WeightRecord>, rusqlite::Error>;
}

impl OptionalRow for rusqlite::Result<WeightRecord> {
    fn optional(self) -> Result<Option<WeightRecord>, rusqlite::Error> {
        match self {
            Ok(r) => Ok(Some(r)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HermesEngine;

    #[test]
    fn test_adjust_weight_reinforce() {
        let engine = HermesEngine::in_memory("test-weight").unwrap();
        let store = WeightStore::new(engine.db().clone());
        let r = store.adjust_weight("node1", 0.1).unwrap();
        assert!(r.weight > 1.0);
        assert_eq!(r.reinforcement_count, 1);
        assert_eq!(r.decay_count, 0);
    }

    #[test]
    fn test_adjust_weight_decay() {
        let engine = HermesEngine::in_memory("test-weight").unwrap();
        let store = WeightStore::new(engine.db().clone());
        let r = store.adjust_weight("node1", -0.2).unwrap();
        assert!(r.weight < 1.0);
        assert_eq!(r.reinforcement_count, 0);
        assert_eq!(r.decay_count, 1);
    }

    #[test]
    fn test_weight_clamped_at_min() {
        let engine = HermesEngine::in_memory("test-weight").unwrap();
        let store = WeightStore::new(engine.db().clone());
        let r = store.adjust_weight("node1", -5.0).unwrap();
        assert_eq!(r.weight, WEIGHT_MIN);
    }

    #[test]
    fn test_weight_clamped_at_max() {
        let engine = HermesEngine::in_memory("test-weight").unwrap();
        let store = WeightStore::new(engine.db().clone());
        let r = store.adjust_weight("node1", 5.0).unwrap();
        assert_eq!(r.weight, WEIGHT_MAX);
    }

    #[test]
    fn test_get_weight_default_is_one() {
        let engine = HermesEngine::in_memory("test-weight").unwrap();
        let store = WeightStore::new(engine.db().clone());
        assert_eq!(store.get_weight("no-such-node").unwrap(), 1.0);
    }

    #[test]
    fn test_get_record_none_when_missing() {
        let engine = HermesEngine::in_memory("test-weight").unwrap();
        let store = WeightStore::new(engine.db().clone());
        assert!(store.get_record("no-such-node").unwrap().is_none());
    }

    #[test]
    fn test_get_weights_for_batch() {
        let engine = HermesEngine::in_memory("test-weight").unwrap();
        let store = WeightStore::new(engine.db().clone());
        store.adjust_weight("n1", 0.1).unwrap();
        store.adjust_weight("n2", -0.1).unwrap();

        let map = store.get_weights_for(&["n1", "n2", "n3"]).unwrap();
        assert_eq!(map.len(), 2);
        assert!(map["n1"] > 1.0);
        assert!(map["n2"] < 1.0);
        assert!(!map.contains_key("n3"));
    }

    #[test]
    fn test_list_non_default_empty_initially() {
        let engine = HermesEngine::in_memory("test-weight").unwrap();
        let store = WeightStore::new(engine.db().clone());
        assert!(store.list_non_default().unwrap().is_empty());
    }

    #[test]
    fn test_list_non_default_shows_adjusted_nodes() {
        let engine = HermesEngine::in_memory("test-weight").unwrap();
        let store = WeightStore::new(engine.db().clone());
        store.adjust_weight("n1", 0.1).unwrap();
        let list = store.list_non_default().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].node_id, "n1");
    }
}
