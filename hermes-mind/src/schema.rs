use anyhow::Result;
use rusqlite::Connection;
use std::sync::{Arc, Mutex};

pub fn run_migrations(db: &Arc<Mutex<Connection>>) -> Result<()> {
    let conn = db.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS connector_state (
            connector  TEXT NOT NULL,
            project_id TEXT NOT NULL,
            key        TEXT NOT NULL,
            value      TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (connector, project_id, key)
        );",
    )?;
    Ok(())
}
