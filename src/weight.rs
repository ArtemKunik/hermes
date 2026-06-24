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

unsafe impl Send for WeightConn {}
unsafe impl Sync for WeightConn {}

impl WeightStore {
    pub fn new(db: Arc<Mutex<Connection>>) -> Self {
        WeightStore { db: WeightConn::Shared(db) }
    }

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
                let conn = unsafe { &**ptr };
                f(conn)
            }
        }
    }

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

    pub fn get_record(&self, node_id: &str) -> Result<Option<WeightRecord>> {
        self.with_conn(|db| {
            match db.query_row(
                "SELECT node_id, weight, reinforcement_count, decay_count, last_updated \
                 FROM weight_index WHERE node_id = ?1",
                params![node_id],
                |row| {
                    Ok(WeightRecord {
                        node_id: row.get(0)?,
                        weight: row.get(1)?,
                        reinforcement_count: row.get(2)?,
                        decay_count: row.get(3)?,
                        last_updated: row.get(4)?,
                    })
                },
            ) {
                Ok(r) => Ok(Some(r)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e.into()),
            }
        })
    }

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

    pub fn adjust_weight(&self, node_id: &str, delta: f64) -> Result<WeightRecord> {
        self.with_conn(|db| {
            let now = chrono::Utc::now().to_rfc3339();

            let current = match db.query_row(
                "SELECT weight, reinforcement_count, decay_count FROM weight_index WHERE node_id = ?1",
                params![node_id],
                |row| Ok((row.get::<_, f64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?)),
            ) {
                Ok(v) => Some(v),
                Err(rusqlite::Error::QueryReturnedNoRows) => None,
                Err(e) => return Err(e.into()),
            };

            let (new_w, new_r, new_d) = match current {
                Some((w, rc, dc)) => {
                    let mut w = w + delta;
                    w = w.clamp(WEIGHT_MIN, WEIGHT_MAX);
                    let (rc, dc) = if delta > 0.0 { (rc + 1, dc) } else { (rc, dc + 1) };
                    (w, rc, dc)
                }
                None => {
                    let w = (1.0 + delta).clamp(WEIGHT_MIN, WEIGHT_MAX);
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
            for r in rows { results.push(r?); }
            Ok(results)
        })
    }

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
            for r in rows { results.push(r?); }
            Ok(results)
        })
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
}
