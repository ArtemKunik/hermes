use anyhow::Result;
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::lock_ext::LockExt;

pub struct HashTracker<'a> {
    db: &'a Arc<Mutex<Connection>>,
    project_id: &'a str,
}

impl<'a> HashTracker<'a> {
    pub fn new(db: &'a Arc<Mutex<Connection>>, project_id: &'a str) -> Self {
        Self { db, project_id }
    }

    pub fn load_all_hashes(&self) -> Result<HashMap<String, String>> {
        let conn = self.db.lock_ctx("hash_tracker")?;
        let mut stmt =
            conn.prepare("SELECT file_path, content_hash FROM file_hashes WHERE project_id = ?1")?;
        let rows = stmt.query_map(params![self.project_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;

        let mut hashes = HashMap::new();
        for row in rows {
            let (file_path, content_hash) = row?;
            hashes.insert(file_path, content_hash);
        }
        Ok(hashes)
    }

    pub fn clear_all_hashes(&self) -> Result<()> {
        let conn = self.db.lock_ctx("hash_tracker")?;
        conn.execute(
            "DELETE FROM file_hashes WHERE project_id = ?1",
            params![self.project_id],
        )?;
        Ok(())
    }

    pub fn store_hash(&self, file_path: &str, hash: &str) -> Result<()> {
        let conn = self.db.lock_ctx("hash_tracker")?;
        conn.execute(
            "INSERT OR REPLACE INTO file_hashes (file_path, project_id, content_hash, indexed_at)
              VALUES (?1, ?2, ?3, datetime('now'))",
            params![file_path, self.project_id, hash],
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
    fn test_load_all_hashes_returns_empty_when_not_stored() {
        use crate::HermesEngine;
        let engine = HermesEngine::in_memory("chunk-test").unwrap();
        let tracker = HashTracker::new(engine.db(), "chunk-test");
        let result = tracker.load_all_hashes().unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_load_all_hashes_includes_stored_chunk_hash() {
        use crate::HermesEngine;
        let engine = HermesEngine::in_memory("chunk-test2").unwrap();
        let tracker = HashTracker::new(engine.db(), "chunk-test2");
        let key = "path/to/file.rs::fn_main";
        let hash = compute_hash("fn main() { println!(\"hello\"); }");
        tracker.store_hash(key, &hash).unwrap();
        let hashes = tracker.load_all_hashes().unwrap();
        assert_eq!(hashes.get(key), Some(&hash));
    }

    #[test]
    fn test_store_hash_overwrites_existing_value() {
        use crate::HermesEngine;
        let engine = HermesEngine::in_memory("chunk-test3").unwrap();
        let tracker = HashTracker::new(engine.db(), "chunk-test3");
        let key = "path/to/file.rs::fn_foo";
        let old_hash = compute_hash("fn foo() {}");
        let new_hash = compute_hash("fn foo() { do_something(); }");
        tracker.store_hash(key, &old_hash).unwrap();
        tracker.store_hash(key, &new_hash).unwrap();
        let hashes = tracker.load_all_hashes().unwrap();
        assert_eq!(hashes.get(key), Some(&new_hash));
    }
}
