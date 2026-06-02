use anyhow::Result;
use rusqlite::params;
use std::path::Path;

const START_MARKER: &str = "<!-- hermes-symbols-start -->";
const END_MARKER: &str = "<!-- hermes-symbols-end -->";

fn kind_abbrev(kind: &str) -> &str {
    match kind {
        "function" => "fn",
        "struct" => "st",
        "enum" => "en",
        "trait" => "tr",
        "impl" => "im",
        "module" => "md",
        "interface" => "if",
        "concept" => "co",
        _ => {
            if kind.len() >= 2 { &kind[..2] } else { kind }
        }
    }
}

fn format_symbol_line(
    file_path: &str,
    symbols: &[SymbolRow],
) -> String {
    let mut line = String::from(file_path);
    line.push_str(": ");
    for (i, s) in symbols.iter().enumerate() {
        if i > 0 {
            line.push(' ');
        }
        line.push_str(&s.name);
        line.push(':');
        line.push_str(kind_abbrev(&s.kind));
        line.push(':');
        line.push_str(&s.line.to_string());
        if let Some(ref methods) = s.methods {
            if !methods.is_empty() {
                line.push('[');
                line.push_str(methods);
                line.push(']');
            }
        }
    }
    line
}

struct SymbolRow {
    name: String,
    kind: String,
    line: i64,
    methods: Option<String>,
    file_path: String,
    exported: bool,
    blast_score: f64,
}

pub fn inject_symbols(
    conn: &rusqlite::Connection,
    project_id: &str,
    target_path: &Path,
    include_all: bool,
    budget: usize,
) -> Result<()> {
    let export_filter = if include_all {
        ""
    } else {
        "AND s.exported = 1"
    };

    let query = format!(
        "SELECT s.name, s.kind, s.line, s.methods, s.file_path, s.exported, \
                COALESCE(bs.blast_score, 0.0) as blast_score \
         FROM symbol_index s \
         LEFT JOIN blast_scores bs ON bs.node_id = s.id AND bs.project_id = s.project_id \
         WHERE s.project_id = ?1 {export_filter} \
         ORDER BY blast_score DESC, s.file_path, s.line"
    );

    let mut stmt = conn.prepare(&query)?;
    let rows = stmt.query_map(params![project_id], |row| {
        Ok(SymbolRow {
            name: row.get(0)?,
            kind: row.get(1)?,
            line: row.get(2)?,
            methods: row.get(3)?,
            file_path: row.get(4)?,
            exported: row.get::<_, i64>(5)? != 0,
            blast_score: row.get(6)?,
        })
    })?;

    let mut all_rows: Vec<SymbolRow> = Vec::new();
    for r in rows {
        all_rows.push(r?);
    }

    let mut lines: Vec<String> = Vec::new();
    let mut token_count = 0usize;

    let mut i = 0;
    while i < all_rows.len() {
        let current_file = all_rows[i].file_path.clone();
        let mut group: Vec<SymbolRow> = Vec::new();
        while i < all_rows.len() && all_rows[i].file_path == current_file {
            group.push(all_rows.remove(i));
        }

        let line = format_symbol_line(&current_file, &group);
        let line_tokens = crate::search::estimate_tokens(&line) as usize;
        if token_count + line_tokens > budget {
            break;
        }
        token_count += line_tokens;
        lines.push(line);
    }

    let content = format!(
        "{START_MARKER}\n{lines}\n{END_MARKER}\n",
        lines = lines.join("\n")
    );

    let existing = std::fs::read_to_string(target_path).unwrap_or_default();

    if let Some(start) = existing.find(START_MARKER) {
        if let Some(end) = existing.find(END_MARKER) {
            let end = end + END_MARKER.len();
            let before = &existing[..start];
            let after = &existing[end..].trim_start();
            let updated = format!("{before}{content}{after}");
            std::fs::write(target_path, updated)?;
            return Ok(());
        }
    }

    let updated = if existing.ends_with('\n') {
        format!("{existing}{content}")
    } else if existing.is_empty() {
        content
    } else {
        format!("{existing}\n{content}")
    };
    std::fs::write(target_path, updated)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use tempfile::TempDir;

    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS symbol_index (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                project_id  TEXT NOT NULL,
                name        TEXT NOT NULL,
                file_path   TEXT NOT NULL,
                line        INTEGER NOT NULL,
                kind        TEXT NOT NULL,
                exported    INTEGER NOT NULL DEFAULT 0,
                methods     TEXT,
                UNIQUE(project_id, name, file_path, line)
            );
            CREATE TABLE IF NOT EXISTS blast_scores (
                node_id          TEXT PRIMARY KEY,
                project_id       TEXT NOT NULL,
                file_path        TEXT,
                direct_count     INTEGER NOT NULL DEFAULT 0,
                transitive_count INTEGER NOT NULL DEFAULT 0,
                blast_score      REAL NOT NULL DEFAULT 0.0,
                risk_level       TEXT NOT NULL DEFAULT 'LOW'
            );",
        ).unwrap();
        conn
    }

    #[test]
    fn test_kind_abbrev() {
        assert_eq!(kind_abbrev("function"), "fn");
        assert_eq!(kind_abbrev("struct"), "st");
        assert_eq!(kind_abbrev("enum"), "en");
        assert_eq!(kind_abbrev("trait"), "tr");
        assert_eq!(kind_abbrev("impl"), "im");
        assert_eq!(kind_abbrev("module"), "md");
        assert_eq!(kind_abbrev("interface"), "if");
        assert_eq!(kind_abbrev("concept"), "co");
        assert_eq!(kind_abbrev("macro"), "ma");
    }

    #[test]
    fn test_format_symbol_line() {
        let symbols = vec![
            SymbolRow {
                name: "verify_token".into(),
                kind: "function".into(),
                line: 42,
                methods: None,
                file_path: "src/auth.rs".into(),
                exported: true,
                blast_score: 10.0,
            },
            SymbolRow {
                name: "AuthService".into(),
                kind: "struct".into(),
                line: 18,
                methods: Some("login,logout".into()),
                file_path: "src/auth.rs".into(),
                exported: true,
                blast_score: 8.0,
            },
        ];
        let line = format_symbol_line("src/auth.rs", &symbols);
        assert_eq!(line, "src/auth.rs: verify_token:fn:42 AuthService:st:18[login,logout]");
    }

    #[test]
    fn test_inject_symbols_creates_file() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO symbol_index (project_id, name, file_path, line, kind, exported)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params!["test", "foo", "src/lib.rs", 1, "function", 1],
        ).unwrap();

        let dir = TempDir::new().unwrap();
        let target = dir.path().join("AGENTS.md");

        inject_symbols(&conn, "test", &target, false, 2000).unwrap();

        let content = std::fs::read_to_string(&target).unwrap();
        assert!(content.contains("hermes-symbols-start"));
        assert!(content.contains("hermes-symbols-end"));
        assert!(content.contains("foo:fn:1"));
    }

    #[test]
    fn test_inject_symbols_idempotent() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO symbol_index (project_id, name, file_path, line, kind, exported)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params!["test", "bar", "src/util.rs", 5, "function", 1],
        ).unwrap();

        let dir = TempDir::new().unwrap();
        let target = dir.path().join("AGENTS.md");

        inject_symbols(&conn, "test", &target, false, 2000).unwrap();
        let first = std::fs::read_to_string(&target).unwrap();
        inject_symbols(&conn, "test", &target, false, 2000).unwrap();
        let second = std::fs::read_to_string(&target).unwrap();

        assert_eq!(first, second);
    }

    #[test]
    fn test_inject_symbols_respects_budget() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO symbol_index (project_id, name, file_path, line, kind, exported)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params!["test", "a", "src/a.rs", 1, "function", 1],
        ).unwrap();
        conn.execute(
            "INSERT INTO symbol_index (project_id, name, file_path, line, kind, exported)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params!["test", "b", "src/b.rs", 2, "function", 1],
        ).unwrap();

        let dir = TempDir::new().unwrap();
        let target = dir.path().join("AGENTS.md");

        inject_symbols(&conn, "test", &target, false, 5).unwrap();
        let content = std::fs::read_to_string(&target).unwrap();
        assert!(content.contains("hermes-symbols-start"));
        assert!(content.contains("hermes-symbols-end"));
    }

    #[test]
    fn test_inject_symbols_excludes_private_by_default() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO symbol_index (project_id, name, file_path, line, kind, exported)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params!["test", "pub_fn", "src/lib.rs", 1, "function", 1],
        ).unwrap();
        conn.execute(
            "INSERT INTO symbol_index (project_id, name, file_path, line, kind, exported)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params!["test", "priv_fn", "src/lib.rs", 5, "function", 0],
        ).unwrap();

        let dir = TempDir::new().unwrap();
        let target = dir.path().join("AGENTS.md");

        inject_symbols(&conn, "test", &target, false, 2000).unwrap();
        let content = std::fs::read_to_string(&target).unwrap();
        assert!(content.contains("pub_fn"));
        assert!(!content.contains("priv_fn"), "private symbols excluded by default");

        inject_symbols(&conn, "test", &target, true, 2000).unwrap();
        let content = std::fs::read_to_string(&target).unwrap();
        assert!(content.contains("priv_fn"), "private symbols included with --all");
    }
}
