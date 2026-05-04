use anyhow::Result;
use rusqlite::{params, Connection};
use std::sync::{Arc, Mutex};

/// Persists per-connector sync cursors (history IDs, delta links, offsets, timestamps)
/// in the connector_state table so each sync only pulls what is new.
pub struct SyncState {
    db: Arc<Mutex<Connection>>,
    project_id: String,
}

impl SyncState {
    pub fn new(db: Arc<Mutex<Connection>>, project_id: impl Into<String>) -> Self {
        Self {
            db,
            project_id: project_id.into(),
        }
    }

    pub fn get(&self, connector: &str, key: &str) -> Result<Option<String>> {
        let conn = self.db.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let val = conn
            .query_row(
                "SELECT value FROM connector_state
                 WHERE connector = ?1 AND project_id = ?2 AND key = ?3",
                params![connector, self.project_id, key],
                |row| row.get::<_, String>(0),
            )
            .ok();
        Ok(val)
    }

    pub fn set(&self, connector: &str, key: &str, value: &str) -> Result<()> {
        let conn = self.db.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        conn.execute(
            "INSERT OR REPLACE INTO connector_state
             (connector, project_id, key, value, updated_at)
             VALUES (?1, ?2, ?3, ?4, datetime('now'))",
            params![connector, self.project_id, key, value],
        )?;
        Ok(())
    }
}
