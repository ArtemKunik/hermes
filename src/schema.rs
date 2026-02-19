use anyhow::Result;
use rusqlite::Connection;

pub fn run_migrations(conn: &Connection) -> Result<()> {
    conn.execute_batch(CREATE_TABLES_SQL)?;
    create_fts_table(conn)?;
    add_accounting_session_id(conn);
    add_name_lower_index(conn);
    Ok(())
}

fn add_name_lower_index(conn: &Connection) {
    let _ = conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_nodes_name_lower ON nodes (LOWER(name));",
    );
}

fn add_accounting_session_id(conn: &Connection) {
    let _ = conn.execute_batch(
        "ALTER TABLE accounting ADD COLUMN session_id TEXT NOT NULL DEFAULT '';",
    );
}

fn create_fts_table(conn: &Connection) -> Result<()> {
    let fts_exists: bool = conn.query_row(
        "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='fts_content'",
        [],
        |row| row.get(0),
    )?;

    if !fts_exists {
        conn.execute_batch(CREATE_FTS_SQL)?;
    }
    Ok(())
}

const CREATE_TABLES_SQL: &str = "
CREATE TABLE IF NOT EXISTS nodes (
    id          TEXT PRIMARY KEY,
    project_id  TEXT NOT NULL,
    name        TEXT NOT NULL,
    node_type   TEXT NOT NULL,
    file_path   TEXT,
    start_line  INTEGER,
    end_line    INTEGER,
    summary     TEXT,
    content_hash TEXT,
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_nodes_project ON nodes(project_id);
CREATE INDEX IF NOT EXISTS idx_nodes_name ON nodes(name);
CREATE INDEX IF NOT EXISTS idx_nodes_type ON nodes(project_id, node_type);
CREATE INDEX IF NOT EXISTS idx_nodes_file ON nodes(file_path);

CREATE TABLE IF NOT EXISTS edges (
    id          TEXT PRIMARY KEY,
    project_id  TEXT NOT NULL,
    source_id   TEXT NOT NULL REFERENCES nodes(id),
    target_id   TEXT NOT NULL REFERENCES nodes(id),
    edge_type   TEXT NOT NULL,
    weight      REAL DEFAULT 1.0,
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(source_id, target_id, edge_type)
);

CREATE INDEX IF NOT EXISTS idx_edges_source ON edges(source_id);
CREATE INDEX IF NOT EXISTS idx_edges_target ON edges(target_id);
CREATE INDEX IF NOT EXISTS idx_edges_project ON edges(project_id);

CREATE TABLE IF NOT EXISTS temporal_facts (
    id                TEXT PRIMARY KEY,
    project_id        TEXT NOT NULL,
    node_id           TEXT REFERENCES nodes(id),
    fact_type         TEXT NOT NULL,
    content           TEXT NOT NULL,
    valid_from        TEXT NOT NULL,
    valid_to          TEXT,
    superseded_by     TEXT,
    source_reference  TEXT,
    created_at        TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_facts_project ON temporal_facts(project_id);
CREATE INDEX IF NOT EXISTS idx_facts_node ON temporal_facts(node_id);
CREATE INDEX IF NOT EXISTS idx_facts_active
    ON temporal_facts(project_id, fact_type) WHERE valid_to IS NULL;

CREATE TABLE IF NOT EXISTS pointer_cache (
    id           TEXT PRIMARY KEY,
    project_id   TEXT NOT NULL,
    node_id      TEXT NOT NULL REFERENCES nodes(id),
    chunk_label  TEXT NOT NULL,
    file_path    TEXT NOT NULL,
    start_line   INTEGER NOT NULL,
    end_line     INTEGER NOT NULL,
    summary      TEXT NOT NULL,
    token_estimate INTEGER NOT NULL DEFAULT 0,
    created_at   TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_pointers_project ON pointer_cache(project_id);
CREATE INDEX IF NOT EXISTS idx_pointers_node ON pointer_cache(node_id);

CREATE TABLE IF NOT EXISTS file_hashes (
    file_path   TEXT PRIMARY KEY,
    project_id  TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    indexed_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS accounting (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id      TEXT NOT NULL,
    session_id      TEXT NOT NULL DEFAULT '',
    query_text      TEXT NOT NULL,
    pointer_tokens  INTEGER NOT NULL DEFAULT 0,
    fetched_tokens  INTEGER NOT NULL DEFAULT 0,
    traditional_est INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_accounting_session ON accounting(project_id, session_id);
";

const CREATE_FTS_SQL: &str = "
CREATE VIRTUAL TABLE fts_content USING fts5(
    node_id,
    project_id,
    name,
    content,
    file_path,
    tokenize='porter unicode61'
);
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_run_without_error() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
    }

    #[test]
    fn migrations_are_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        run_migrations(&conn).unwrap();
    }

    #[test]
    fn fts_table_created() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='fts_content'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }
}
