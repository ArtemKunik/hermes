// ChartApp/hermes-engine/src/ingestion/hash_tracker.rs
use anyhow::Result;
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::{Arc, Mutex};

const INDEX_HASH_VERSION: &str = "index-v2";

pub struct HashTracker<'a> {
    db: &'a Arc<Mutex<Connection>>,
    project_id: &'a str,
}

pub fn normalize_logical_path(path: &str) -> String {
    path.replace('\\', "/")
}

impl<'a> HashTracker<'a> {
    pub fn new(db: &'a Arc<Mutex<Connection>>, project_id: &'a str) -> Self {
        Self { db, project_id }
    }

    pub fn is_unchanged(&self, file_path: &str) -> Result<bool> {
        let file_path = normalize_logical_path(file_path);
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

        // Lossy decode matches the behaviour of ingest_file so hashes are consistent.
        let raw_bytes = std::fs::read(file_path)?;
        let content = String::from_utf8_lossy(&raw_bytes).into_owned();
        let current_hash = compute_hash(&content);
        Ok(stored == current_hash)
    }

    /// Load all stored hashes for this project in a single query.
    ///
    /// Returns a map of `file_path → content_hash`. Callers can then classify
    /// changed vs unchanged files entirely in memory without holding the Mutex.
    pub fn load_all_hashes(&self) -> Result<std::collections::HashMap<String, String>> {
        let conn = self.db.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut stmt = conn.prepare(
            "SELECT file_path, content_hash FROM file_hashes WHERE project_id = ?1",
        )?;
        let rows = stmt
            .query_map(params![self.project_id], |row| {
                Ok((
                    normalize_logical_path(&row.get::<_, String>(0)?),
                    row.get::<_, String>(1)?,
                ))
            })?
            .collect::<Result<std::collections::HashMap<_, _>, _>>()?;
        Ok(rows)
    }

    pub fn update_hash(&self, file_path: &str, actual_path: &Path) -> Result<()> {
        let file_path = normalize_logical_path(file_path);
        let raw_bytes = std::fs::read(actual_path)?;
        let content = String::from_utf8_lossy(&raw_bytes).into_owned();
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
        let chunk_key = normalize_logical_path(chunk_key);
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
        let chunk_key = normalize_logical_path(chunk_key);
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
    compute_hash_with_version(content, INDEX_HASH_VERSION)
}

fn compute_hash_with_version(content: &str, version: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(version.as_bytes());
    hasher.update([0u8]);
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
    fn normalize_logical_path_converts_backslashes() {
        assert_eq!(
            normalize_logical_path(r"D:\repo\ChartApp\file.rs"),
            "D:/repo/ChartApp/file.rs"
        );
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
    fn hash_includes_index_version_salt() {
        let plain = {
            let mut hasher = Sha256::new();
            hasher.update("hello".as_bytes());
            hex::encode(hasher.finalize())
        };
        let versioned = compute_hash("hello");
        assert_ne!(plain, versioned);
    }

    #[test]
    fn test_chunk_unchanged_returns_false_when_not_stored() {
        use crate::HermesEngine;
        let engine = HermesEngine::in_memory("chunk-test").unwrap();
        let tracker = HashTracker::new(engine.db(), "chunk-test");
        let result = tracker
            .is_chunk_unchanged("path/to/file.rs::fn_name", "abc123")
            .unwrap();
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
    fn load_all_hashes_returns_empty_for_fresh_project() {
        use crate::HermesEngine;
        let engine = HermesEngine::in_memory("load-all-empty").unwrap();
        let tracker = HashTracker::new(engine.db(), "load-all-empty");
        let hashes = tracker.load_all_hashes().unwrap();
        assert!(hashes.is_empty());
    }

    #[test]
    fn load_all_hashes_returns_stored_entries() {
        use crate::HermesEngine;
        let engine = HermesEngine::in_memory("load-all-entries").unwrap();
        let tracker = HashTracker::new(engine.db(), "load-all-entries");

        let temp = tempfile::tempdir().unwrap();
        let path_a = temp.path().join("a.rs");
        let path_b = temp.path().join("b.rs");
        std::fs::write(&path_a, b"fn a() {}").unwrap();
        std::fs::write(&path_b, b"fn b() {}").unwrap();

        tracker.update_hash(path_a.to_str().unwrap(), &path_a).unwrap();
        tracker.update_hash(path_b.to_str().unwrap(), &path_b).unwrap();

        let hashes = tracker.load_all_hashes().unwrap();
        assert_eq!(hashes.len(), 2);
        assert!(hashes.contains_key(&normalize_logical_path(path_a.to_str().unwrap())));
        assert!(hashes.contains_key(&normalize_logical_path(path_b.to_str().unwrap())));
    }

    #[test]
    fn load_all_hashes_is_project_scoped() {
        use crate::HermesEngine;
        let engine = HermesEngine::in_memory("load-all-scope").unwrap();
        let tracker_a = HashTracker::new(engine.db(), "project-a");
        let tracker_b = HashTracker::new(engine.db(), "project-b");

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("file.rs");
        std::fs::write(&path, b"fn x() {}").unwrap();

        tracker_a.update_hash(path.to_str().unwrap(), &path).unwrap();

        let hashes_a = tracker_a.load_all_hashes().unwrap();
        let hashes_b = tracker_b.load_all_hashes().unwrap();
        assert_eq!(hashes_a.len(), 1);
        assert!(hashes_b.is_empty());
    }

    #[test]
    fn update_hash_stores_normalized_path_key() {
        use crate::HermesEngine;
        let engine = HermesEngine::in_memory("normalized-hash-key").unwrap();
        let tracker = HashTracker::new(engine.db(), "normalized-hash-key");

        let temp = tempfile::tempdir().unwrap();
        let actual_path = temp.path().join("file.rs");
        std::fs::write(&actual_path, b"fn normalized() {}").unwrap();

        tracker.update_hash(r"dir\file.rs", &actual_path).unwrap();

        let hashes = tracker.load_all_hashes().unwrap();
        assert!(hashes.contains_key("dir/file.rs"));
        assert!(!hashes.contains_key(r"dir\file.rs"));
    }

    #[test]
    fn chunk_hash_lookup_treats_slashes_and_backslashes_as_same_key() {
        use crate::HermesEngine;
        let engine = HermesEngine::in_memory("normalized-chunk-key").unwrap();
        let tracker = HashTracker::new(engine.db(), "normalized-chunk-key");
        let hash = compute_hash("fn main() {}");

        tracker.update_chunk_hash(r"dir\file.rs::fn_main", &hash).unwrap();

        assert!(tracker
            .is_chunk_unchanged("dir/file.rs::fn_main", &hash)
            .unwrap());
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
