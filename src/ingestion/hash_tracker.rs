// ChartApp/hermes-engine/src/ingestion/hash_tracker.rs
use anyhow::Result;
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::{Arc, Mutex};

pub struct HashTracker<'a> {
    db: &'a Arc<Mutex<Connection>>,
    project_id: &'a str,
}

impl<'a> HashTracker<'a> {
    pub fn new(db: &'a Arc<Mutex<Connection>>, project_id: &'a str) -> Self {
        Self { db, project_id }
    }

    pub fn is_unchanged(&self, file_path: &str) -> Result<bool> {
        let conn = self.db.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let stored_hash: Option<String> = conn
            .query_row(
                "SELECT content_hash FROM file_hashes WHERE file_path = ?1 AND project_id = ?2",
                params![file_path, self.project_id],
                |row| row.get(0),
            )
            .ok();

        let Some(stored) = stored_hash else {
            return Ok(false);
        };

        let content = std::fs::read_to_string(file_path)?;
        let current_hash = compute_hash(&content);
        Ok(stored == current_hash)
    }

    pub fn update_hash(&self, file_path: &str, actual_path: &Path) -> Result<()> {
        let content = std::fs::read_to_string(actual_path)?;
        let hash = compute_hash(&content);
        let conn = self.db.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        conn.execute(
            "INSERT OR REPLACE INTO file_hashes (file_path, project_id, content_hash, indexed_at)
             VALUES (?1, ?2, ?3, datetime('now'))",
            params![file_path, self.project_id, hash],
        )?;
        Ok(())
    }

    /// Task 2.2: Returns true if the chunk's content hash matches what is stored.
    /// `chunk_key` is a stable identifier combining file_path + chunk name.
    pub fn is_chunk_unchanged(&self, chunk_key: &str, current_hash: &str) -> Result<bool> {
        let conn = self.db.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let stored: Option<String> = conn
            .query_row(
                "SELECT content_hash FROM file_hashes WHERE file_path = ?1 AND project_id = ?2",
                params![chunk_key, self.project_id],
                |row| row.get(0),
            )
            .ok();
        Ok(stored.as_deref() == Some(current_hash))
    }

    /// Task 2.2: Persist the chunk hash so subsequent ingestion runs can skip unchanged chunks.
    pub fn update_chunk_hash(&self, chunk_key: &str, hash: &str) -> Result<()> {
        let conn = self.db.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        conn.execute(
            "INSERT OR REPLACE INTO file_hashes (file_path, project_id, content_hash, indexed_at)
             VALUES (?1, ?2, ?3, datetime('now'))",
            params![chunk_key, self.project_id, hash],
        )?;
        Ok(())
    }
}

pub fn compute_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_deterministic() {
        let h1 = compute_hash("hello world");
        let h2 = compute_hash("hello world");
        assert_eq!(h1, h2);
    }

    #[test]
    fn different_content_different_hash() {
        let h1 = compute_hash("hello");
        let h2 = compute_hash("world");
        assert_ne!(h1, h2);
    }

    #[test]
    fn hash_is_64_hex_chars() {
        let h = compute_hash("test");
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_chunk_unchanged_returns_false_when_not_stored() {
        use crate::HermesEngine;
        let engine = HermesEngine::in_memory("chunk-test").unwrap();
        let tracker = HashTracker::new(engine.db(), "chunk-test");
        let result = tracker.is_chunk_unchanged("path/to/file.rs::fn_name", "abc123").unwrap();
        assert!(!result);
    }

    #[test]
    fn test_chunk_unchanged_returns_true_after_store() {
        use crate::HermesEngine;
        let engine = HermesEngine::in_memory("chunk-test2").unwrap();
        let tracker = HashTracker::new(engine.db(), "chunk-test2");
        let key = "path/to/file.rs::fn_main";
        let hash = compute_hash("fn main() { println!(\"hello\"); }");
        tracker.update_chunk_hash(key, &hash).unwrap();
        assert!(tracker.is_chunk_unchanged(key, &hash).unwrap());
    }

    #[test]
    fn test_chunk_changed_returns_false_on_different_hash() {
        use crate::HermesEngine;
        let engine = HermesEngine::in_memory("chunk-test3").unwrap();
        let tracker = HashTracker::new(engine.db(), "chunk-test3");
        let key = "path/to/file.rs::fn_foo";
        let old_hash = compute_hash("fn foo() {}");
        let new_hash = compute_hash("fn foo() { do_something(); }");
        tracker.update_chunk_hash(key, &old_hash).unwrap();
        assert!(!tracker.is_chunk_unchanged(key, &new_hash).unwrap());
    }
}

