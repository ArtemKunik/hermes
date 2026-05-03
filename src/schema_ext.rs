// tools/hermes-engine/src/schema_ext.rs
//
// Extension migrations added after the initial schema.

use rusqlite::Connection;

/// Idempotent: create table to store indexed skill metadata (TRACK-041 Phase 2).
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

/// Idempotent: creates search_misses table for post-mortem analysis of zero-result queries.
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

/// Idempotent: add new columns to temporal_facts for enriched fact API.
pub(crate) fn extend_temporal_facts_columns(conn: &Connection) {
    let cols = [
        "ALTER TABLE temporal_facts ADD COLUMN topic TEXT;",
        "ALTER TABLE temporal_facts ADD COLUMN tags TEXT;",
        "ALTER TABLE temporal_facts ADD COLUMN confidence REAL;",
        "ALTER TABLE temporal_facts ADD COLUMN provenance TEXT;",
        "ALTER TABLE temporal_facts ADD COLUMN repo_id TEXT;",
        "ALTER TABLE temporal_facts ADD COLUMN agent_id TEXT;",
    ];
    for ddl in &cols {
        let _ = conn.execute_batch(ddl);
    }
    let _ = conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_facts_topic ON temporal_facts(project_id, topic);
         CREATE INDEX IF NOT EXISTS idx_facts_repo ON temporal_facts(repo_id);
         CREATE INDEX IF NOT EXISTS idx_facts_agent ON temporal_facts(agent_id);
         CREATE INDEX IF NOT EXISTS idx_facts_valid_to ON temporal_facts(valid_to);",
    );
}

/// Idempotent: create missions + mission_log tables for mission lifecycle.
pub(crate) fn create_missions_tables(conn: &Connection) {
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS missions (
            id          TEXT PRIMARY KEY,
            project_id  TEXT NOT NULL,
            title       TEXT NOT NULL,
            description TEXT,
            status      TEXT NOT NULL DEFAULT 'preflight',
            tags        TEXT,
            checklist   TEXT,
            diff        TEXT,
            commit_range TEXT,
            repo_id     TEXT,
            agent_id    TEXT,
            created_at  TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_missions_project ON missions(project_id);
        CREATE INDEX IF NOT EXISTS idx_missions_status ON missions(project_id, status);
        CREATE INDEX IF NOT EXISTS idx_missions_repo ON missions(repo_id);
        CREATE INDEX IF NOT EXISTS idx_missions_agent ON missions(agent_id);

        CREATE TABLE IF NOT EXISTS mission_log (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            mission_id  TEXT NOT NULL REFERENCES missions(id),
            event_type  TEXT NOT NULL,
            data        TEXT,
            created_at  TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_mission_log_mission ON mission_log(mission_id);",
    );
}
