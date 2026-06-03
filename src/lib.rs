// tools/hermes-engine/src/lib.rs
#![recursion_limit = "256"]
pub mod accounting;
pub mod arch_rules;
pub mod blast_radius;
pub mod hook;
pub mod symbol_index;
pub mod symbol_inject;
pub mod embedding;
pub mod graph;
pub mod graph_builders;
pub mod graph_ops;
pub mod graph_queries;
pub mod graph_support;
pub mod graph_types;
pub mod http_api;
pub mod index_lock;
pub mod ingestion;
pub mod mcp_actor;
pub mod mcp_compaction;
mod mcp_compaction_support;
pub mod memory_indexer;
pub mod memory_query;
pub mod mcp_commit;
pub mod mcp_constraints;
pub mod mcp_coverage;
pub mod mcp_heal;
pub mod mcp_incidents;
pub mod incident_io;
pub mod kb_handler;
pub mod mcp_lint;
pub mod mcp_dismiss;
pub mod mcp_lint_scope;
pub mod mcp_memory;
pub mod mission;
pub mod mcp_missions;
pub mod proposal;
pub mod mcp_proposals;
pub mod mcp_quality;
pub mod mcp_quality_drift;
pub mod mcp_server;
pub mod quality;
pub mod mcp_tool_schemas;
pub mod mcp_skills;
pub mod mcp_status;
pub mod mcp_tracks;
mod mcp_tracks_support;
pub mod mcp_tools;
pub mod mcp_tools_analysis;
pub mod mcp_tools_graph;
pub mod mcp_tools_stats;
pub mod mcp_tools_consistency;
pub mod mcp_tools_validation;
pub mod mcp_wire;
pub mod pointer;
pub mod retry;
pub mod schema;
pub mod search;
pub mod temporal;
pub mod temporal_types;
pub mod tool_circuit_breaker;
pub mod tool_runtime;
pub mod tool_router;
pub mod watcher;
pub mod weight;
pub mod viz;

use crate::pointer::PointerResponse;
use crate::tool_circuit_breaker::ToolCircuitBreaker;
use anyhow::Result;
use rusqlite::Connection;
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// In-process search result cache entry: (response, time_inserted).
pub type SearchCacheMap = HashMap<String, (PointerResponse, Instant)>;

#[derive(Clone)]
pub struct HermesEngine {
    db: Arc<Mutex<Connection>>,
    /// Dedicated read-only connection for diagnostic tools (mcp_status, mcp_stats).
    /// Separate from `db` so WAL-mode concurrent reads proceed without blocking
    /// the write lock held by the auto-indexer during long ingestion runs.
    /// This connection uses a short busy timeout so diagnostic tools fail fast
    /// instead of hanging until the MCP actor times out.
    read_db: Arc<Mutex<Connection>>,
    db_path: Option<PathBuf>,
    project_id: String,
    /// Unique ID for this process invocation. Stored on each accounting row
    /// for per-run traceability; session-window stats now use a daily UTC window.
    session_id: String,
    /// Task 1.3: In-process LRU-style search result cache (60s TTL, max 256 entries).
    /// Keyed on "query_lower:top_k". Shared across SearchEngine instances via clone of Arc.
    search_cache: Arc<Mutex<SearchCacheMap>>,
    pending_calls: Arc<AtomicUsize>,
    tool_circuit_breaker: ToolCircuitBreaker,
    /// TRACK-066: Singleton process guard.
    singleton: Option<Arc<HermesSingleton>>,
}

/// TRACK-066: Singleton process guard to prevent multiple Hermes processes
/// from competing on the same database.
pub struct HermesSingleton {
    pid_path: PathBuf,
}

impl HermesSingleton {
    pub fn try_acquire(project_root: &Path) -> Option<Self> {
        let pid_path = project_root.join(".hermes.pid");
        if let Ok(existing_pid_str) = std::fs::read_to_string(&pid_path) {
            if let Ok(pid) = existing_pid_str.trim().parse::<u32>() {
                if crate::index_lock::process_is_alive(pid) && pid != std::process::id() {
                    return None;
                }
            }
        }
        let _ = std::fs::write(&pid_path, std::process::id().to_string());
        Some(Self { pid_path })
    }
}

impl Drop for HermesSingleton {
    fn drop(&mut self) {
        if let Ok(pid_str) = std::fs::read_to_string(&self.pid_path) {
            if pid_str.trim() == std::process::id().to_string() {
                let _ = std::fs::remove_file(&self.pid_path);
            }
        }
    }
}

/// SQLite PRAGMAs applied to every connection.
///
/// - WAL mode: concurrent readers don't block the single writer.
/// - NORMAL synchronous: safe durability without a full fsync on every write.
/// - cache_size=-65536: 64 MB page cache in RAM (avoids repeated disk reads).
/// - temp_store=MEMORY: in-memory temp tables (faster sorts / FTS scratch).
/// - mmap_size: 256 MB memory-mapped I/O (no-op on Windows, helps Linux).
const PERF_PRAGMAS: &str = "\
    PRAGMA journal_mode=WAL;\
    PRAGMA synchronous=NORMAL;\
    PRAGMA cache_size=-65536;\
    PRAGMA temp_store=MEMORY;\
    PRAGMA mmap_size=268435456;\
";

impl HermesEngine {
    pub fn new(db_path: &Path, project_id: &str) -> Result<Self> {
        let conn = Connection::open(db_path)?;
        // Use WAL to allow concurrent readers and a single writer, and
        // relaxed synchronous mode for performance.  Also configure a busy
        // timeout so that transient locks (e.g. another process briefly
        // writing) don't immediately bubble up as "database is locked".  The
        // default is 30 seconds but can be overridden with the optional
        // HERMES_DB_BUSY_TIMEOUT environment variable (seconds).
        conn.execute_batch(PERF_PRAGMAS)?;
        let timeout_secs = resolve_busy_timeout_secs();
        conn.busy_timeout(Duration::from_secs(timeout_secs))?;
        schema::run_migrations(&conn)?;
        // Second connection for read-only diagnostic queries. WAL mode allows
        // this connection to read concurrently while the write connection runs
        // long ingestion transactions.
        let read_conn = Connection::open(db_path)?;
        read_conn.execute_batch(PERF_PRAGMAS)?;
        read_conn.busy_timeout(Duration::from_millis(resolve_diagnostic_busy_timeout_ms()))?;

        // Try to acquire singleton status if we have a real project root.
        let project_root = db_path.parent().unwrap_or_else(|| Path::new("."));
        let singleton = HermesSingleton::try_acquire(project_root).map(Arc::new);

        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
            read_db: Arc::new(Mutex::new(read_conn)),
            db_path: Some(db_path.to_path_buf()),
            project_id: project_id.to_string(),
            session_id: Uuid::new_v4().to_string(),
            search_cache: Arc::new(Mutex::new(HashMap::new())),
            pending_calls: Arc::new(AtomicUsize::new(0)),
            tool_circuit_breaker: ToolCircuitBreaker::new(),
            singleton,
        })
    }

    pub fn in_memory(project_id: &str) -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(PERF_PRAGMAS)?;
        let timeout_secs = resolve_busy_timeout_secs();
        conn.busy_timeout(Duration::from_secs(timeout_secs))?;
        schema::run_migrations(&conn)?;
        // In-memory DBs are single-connection by nature; share the Arc so
        // read_db() works correctly in tests without a second database.
        let db = Arc::new(Mutex::new(conn));
        Ok(Self {
            db: db.clone(),
            read_db: db,
            db_path: None,
            project_id: project_id.to_string(),
            session_id: Uuid::new_v4().to_string(),
            search_cache: Arc::new(Mutex::new(HashMap::new())),
            pending_calls: Arc::new(AtomicUsize::new(0)),
            tool_circuit_breaker: ToolCircuitBreaker::new(),
            singleton: None,
        })
    }

    pub fn db(&self) -> &Arc<Mutex<Connection>> {
        &self.db
    }

    /// Returns the dedicated read-only connection for diagnostic tools.
    /// Use this in `mcp_status` and `mcp_stats` instead of `db()` so that
    /// long write transactions (auto-index) do not cause tool timeouts.
    pub fn read_db(&self) -> &Arc<Mutex<Connection>> {
        &self.read_db
    }

    /// Returns a fresh diagnostic connection for file-backed databases so
    /// stats/status tools do not contend on a shared mutex when an earlier
    /// diagnostic call is already blocked inside SQLite. In-memory tests fall
    /// back to the shared connection because there is no second database.
    pub fn diagnostic_db(&self) -> Result<Cow<'_, Arc<Mutex<Connection>>>> {
        match &self.db_path {
            Some(path) => {
                let conn = Connection::open(path)?;
                conn.execute_batch(PERF_PRAGMAS)?;
                conn.busy_timeout(Duration::from_millis(resolve_diagnostic_busy_timeout_ms()))?;
                Ok(Cow::Owned(Arc::new(Mutex::new(conn))))
            }
            None => Ok(Cow::Borrowed(&self.read_db)),
        }
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

    pub fn pending_calls(&self) -> Arc<AtomicUsize> {
        self.pending_calls.clone()
    }

    pub fn tool_circuit_breaker(&self) -> ToolCircuitBreaker {
        self.tool_circuit_breaker.clone()
    }

    pub fn duplicate_process_detected(&self) -> bool {
        self.singleton.is_none() && self.db_path.is_some()
    }

    /// Task 1.3: Invalidate the search cache (called after re-index).
    pub fn invalidate_search_cache(&self) {
        if let Ok(mut cache) = self.search_cache.lock() {
            cache.clear();
        }
    }
}

pub fn resolve_busy_timeout_secs() -> u64 {
    std::env::var("HERMES_DB_BUSY_TIMEOUT")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(60)
}

pub fn resolve_diagnostic_busy_timeout_ms() -> u64 {
    std::env::var("HERMES_DIAGNOSTIC_DB_BUSY_TIMEOUT_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(500)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod mcp_actor_tests;

#[cfg(test)]
mod mcp_tracks_tests;

#[cfg(test)]
mod mcp_server_tests;
