use rusqlite::Connection;

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

pub(crate) fn migrate_temporal_facts_extended(conn: &Connection) {
    for (col, def) in &[
        ("topic", "TEXT"),
        ("tags", "TEXT NOT NULL DEFAULT '[]'"),
        ("confidence", "REAL"),
        ("provenance", "TEXT"),
        ("repo_id", "TEXT"),
        ("agent_id", "TEXT"),
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

pub(crate) fn add_accounting_memory_hits(conn: &Connection) {
    let _ = conn.execute_batch(
        "ALTER TABLE accounting ADD COLUMN memory_hits INTEGER NOT NULL DEFAULT 0;",
    );
}
