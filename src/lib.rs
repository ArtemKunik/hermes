// tools/hermes-engine/src/lib.rs
pub mod accounting;
pub mod embedding;
pub mod mcp_server;
pub mod graph;
pub mod graph_builders;
pub mod graph_queries;
pub mod ingestion;
pub mod pointer;
pub mod schema;
pub mod search;
pub mod temporal;

use anyhow::Result;
use crate::pointer::PointerResponse;
use rusqlite::Connection;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use uuid::Uuid;

/// In-process search result cache entry: (response, time_inserted).
pub type SearchCacheMap = HashMap<String, (PointerResponse, Instant)>;

#[derive(Clone)]
pub struct HermesEngine {
    db: Arc<Mutex<Connection>>,
    project_id: String,
    /// Unique ID for this process invocation. Used to scope per-session accounting.
    session_id: String,
    /// Task 1.3: In-process LRU-style search result cache (60s TTL, max 256 entries).
    /// Keyed on "query_lower:top_k". Shared across SearchEngine instances via clone of Arc.
    search_cache: Arc<Mutex<SearchCacheMap>>,
}

impl HermesEngine {
    pub fn new(db_path: &Path, project_id: &str) -> Result<Self> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        schema::run_migrations(&conn)?;
        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
            project_id: project_id.to_string(),
            session_id: Uuid::new_v4().to_string(),
            search_cache: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub fn in_memory(project_id: &str) -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        schema::run_migrations(&conn)?;
        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
            project_id: project_id.to_string(),
            session_id: Uuid::new_v4().to_string(),
            search_cache: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub fn db(&self) -> &Arc<Mutex<Connection>> {
        &self.db
    }

    pub fn project_id(&self) -> &str {
        &self.project_id
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Returns the shared search result cache. Pass this into SearchEngine::new().
    pub fn search_cache(&self) -> Arc<Mutex<SearchCacheMap>> {
        self.search_cache.clone()
    }

    /// Task 1.3: Invalidate the search cache (called after re-index).
    pub fn invalidate_search_cache(&self) {
        if let Ok(mut cache) = self.search_cache.lock() {
            cache.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_in_memory_engine() {
        let engine = HermesEngine::in_memory("test-project").unwrap();
        assert_eq!(engine.project_id(), "test-project");
    }

    #[test]
    fn search_cache_starts_empty() {
        let engine = HermesEngine::in_memory("test-cache").unwrap();
        let cache_arc = engine.search_cache();
        let guard = cache_arc.lock().unwrap();
        assert!(guard.is_empty());
    }

    #[test]
    fn invalidate_clears_cache() {
        let engine = HermesEngine::in_memory("test-inv").unwrap();
        {
            let cache_arc = engine.search_cache();
            let mut cache = cache_arc.lock().unwrap();
            let dummy = PointerResponse::build(vec![], 0);
            cache.insert("key".to_string(), (dummy, Instant::now()));
        }
        engine.invalidate_search_cache();
        let cache_arc = engine.search_cache();
        let cache = cache_arc.lock().unwrap();
        assert!(cache.is_empty());
    }
}
