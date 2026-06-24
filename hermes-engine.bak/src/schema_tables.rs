// hermes-engine/src/schema_tables.rs — table creation helpers (SIZE-001 split).
use rusqlite::Connection;

/// Idempotent: create table to store symbol embeddings for duplicate detection.
pub(crate) fn add_symbol_embeddings_table(conn: &Connection) {
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS symbol_embeddings (
            id          TEXT PRIMARY KEY,
            symbol_name TEXT NOT NULL,
            file_path   TEXT NOT NULL,
            language    TEXT NOT NULL,
            signature   TEXT NOT NULL,
            snippet     TEXT NOT NULL,
            embedding   BLOB NOT NULL,
            indexed_at  DATETIME DEFAULT CURRENT_TIMESTAMP
        );",
    );
}

/// Idempotent: creates the weight_index table for AD-02 reinforcement learning.
pub(crate) fn add_weight_index(conn: &Connection) {
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS weight_index (
            node_id              TEXT PRIMARY KEY,
            weight               REAL    NOT NULL DEFAULT 1.0,
            reinforcement_count  INTEGER NOT NULL DEFAULT 0,
            decay_count          INTEGER NOT NULL DEFAULT 0,
            last_updated         TEXT    NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_weight_index_weight ON weight_index(weight);",
    );
}

/// Idempotent: creates dedicated table for memory usage statistics.
pub(crate) fn create_memory_stats_table(conn: &Connection) {
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS memory_stats (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            project_id      TEXT NOT NULL,
            session_id      TEXT NOT NULL DEFAULT '',
            event_type      TEXT NOT NULL,
            topic           TEXT,
            file_path       TEXT,
            tags            TEXT,
            created_at      TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_memory_stats_project
            ON memory_stats(project_id);
        CREATE INDEX IF NOT EXISTS idx_memory_stats_event
            ON memory_stats(project_id, event_type);",
    );
}

/// Idempotent: creates config_registry table for environment variable tracking.
pub(crate) fn create_config_registry_table(conn: &Connection) {
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS config_registry (
            key         TEXT PRIMARY KEY,
            value       TEXT NOT NULL,
            source      TEXT NOT NULL,
            created_at  TEXT NOT NULL DEFAULT (datetime('now'))
        );",
    );
}

/// Idempotent: create table to store indexed skill metadata.
pub(crate) fn create_skills_table(conn: &Connection) {
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS skills (
            id          TEXT PRIMARY KEY,
            project_id  TEXT NOT NULL,
            name        TEXT NOT NULL,
            description TEXT,
            category    TEXT,
            language    TEXT,
            version     TEXT,
            file_path   TEXT NOT NULL,
            scope       TEXT NOT NULL DEFAULT 'project',
            tags        TEXT,
            created_at  TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_skills_project ON skills(project_id);
        CREATE INDEX IF NOT EXISTS idx_skills_name ON skills(project_id, name);",
    );
}

/// Idempotent: creates search_misses table for zero-result query analysis.
pub(crate) fn create_search_misses_table(conn: &Connection) {
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS search_misses (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            project_id      TEXT NOT NULL,
            session_id      TEXT NOT NULL DEFAULT '',
            query           TEXT NOT NULL,
            effective_query TEXT,
            goal            TEXT,
            source          TEXT NOT NULL DEFAULT 'mcp',
            created_at      TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_search_misses_project
            ON search_misses(project_id);
        CREATE INDEX IF NOT EXISTS idx_search_misses_query
            ON search_misses(project_id, query);",
    );
}

/// Idempotent: add extended columns to temporal_facts (topic, tags, confidence, …).
pub(crate) fn migrate_temporal_facts_extended(conn: &Connection) {
    for (col, def) in &[
        ("topic",       "TEXT"),
        ("tags",        "TEXT NOT NULL DEFAULT '[]'"),
        ("confidence",  "REAL"),
        ("provenance",  "TEXT"),
        ("repo_id",     "TEXT"),
        ("agent_id",    "TEXT"),
    ] {
        let _ = conn.execute_batch(&format!(
            "ALTER TABLE temporal_facts ADD COLUMN {col} {def};"
        ));
    }
    let _ = conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_facts_topic \
         ON temporal_facts(project_id, topic) WHERE valid_to IS NULL;",
    );
    let _ = conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_facts_repo \
         ON temporal_facts(project_id, repo_id) WHERE valid_to IS NULL;",
    );
}

/// Idempotent: create `missions` table for the hermes-missions spec.
pub(crate) fn create_missions_table(conn: &Connection) {
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS missions (
            id           TEXT PRIMARY KEY,
            project_id   TEXT NOT NULL,
            title        TEXT NOT NULL,
            description  TEXT,
            status       TEXT NOT NULL DEFAULT 'preflight',
            tags         TEXT NOT NULL DEFAULT '[]',
            checklist    TEXT NOT NULL DEFAULT '[]',
            log          TEXT NOT NULL DEFAULT '[]',
            repo_id      TEXT,
            agent_id     TEXT,
            created_at   TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at   TEXT NOT NULL DEFAULT (datetime('now')),
            completed_at TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_missions_project
            ON missions(project_id);
        CREATE INDEX IF NOT EXISTS idx_missions_status
            ON missions(project_id, status);
        CREATE INDEX IF NOT EXISTS idx_missions_repo
            ON missions(project_id, repo_id);

        CREATE TABLE IF NOT EXISTS mission_log (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            mission_id  TEXT NOT NULL REFERENCES missions(id),
            event_type  TEXT NOT NULL,
            data        TEXT,
            created_at  TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_mission_log_mission
            ON mission_log(mission_id);",
    );

    let _ = conn.execute_batch("ALTER TABLE missions ADD COLUMN diff TEXT;");
    let _ = conn.execute_batch("ALTER TABLE missions ADD COLUMN commit_range TEXT;");
}
