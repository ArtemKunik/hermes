use super::*;
use rusqlite::Connection;

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
