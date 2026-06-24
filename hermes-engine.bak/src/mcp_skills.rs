// tools/hermes-engine/src/mcp_skills.rs
//
// TRACK-041 Phase 2: Skill Discovery & Retrieval — MCP tools
//
// `hermes_match_skills`  — ranked search over the `skills` table
// `hermes_fetch_skill`   — return full skill content + resource roots

use anyhow::Result;
use rusqlite::params;
use serde_json::{json, Value};

use crate::accounting::Accountant;

/// Search the skills table for entries matching `query`.
/// Returns ranked results; optional `scope` filter ("project" | "shared").
pub fn tool_match_skills(
    engine: &crate::HermesEngine,
    query: &str,
    scope: Option<&str>,
) -> Result<Value> {
    let db = engine.read_db().lock().unwrap_or_else(|e| e.into_inner());
    tool_match_skills_with_conn(engine, &db, query, scope)
}

pub fn tool_match_skills_with_conn(
    engine: &crate::HermesEngine,
    conn: &rusqlite::Connection,
    query: &str,
    scope: Option<&str>,
) -> Result<Value> {
    let mut sql = String::from(
        "SELECT id, name, description, category, language, version, file_path, scope, tags \
         FROM skills WHERE project_id = ?1",
    );
    if let Some(sc) = scope {
        sql.push_str(&format!(" AND scope = '{}'", sc.replace('\'', "''")));
    }

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![engine.project_id()], |row| {
        Ok(SkillRow {
            id: row.get(0)?,
            name: row.get(1)?,
            description: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
            category: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
            language: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
            version: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
            file_path: row.get(6)?,
            scope: row.get(7)?,
            tags: row.get::<_, Option<String>>(8)?.unwrap_or_default(),
        })
    })?;

    let terms: Vec<&str> = query.split_whitespace().collect();

    let mut scored: Vec<(f64, SkillRow)> = Vec::new();
    for row in rows.flatten() {
        let haystack = format!(
            "{} {} {} {} {}",
            row.name, row.description, row.category, row.tags, row.language
        )
        .to_lowercase();

        let mut score: f64 = 0.0;
        for term in &terms {
            if haystack.contains(&term.to_lowercase()) {
                score += 1.0;
            }
        }
        if score > 0.0 {
            scored.push((score, row));
        }
    }
    drop(stmt);

    // Sort by scope precedence (project=0, shared=1, global=2), then score descending
    scored.sort_by(|a, b| {
        let sa = scope_precedence(&a.1.scope);
        let sb = scope_precedence(&b.1.scope);
        sa.cmp(&sb).then(b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal))
    });

    let matches: Vec<Value> = scored
        .iter()
        .take(10)
        .map(|(score, row)| {
            json!({
                "id": row.id,
                "name": row.name,
                "description": row.description,
                "category": row.category,
                "language": row.language,
                "version": row.version,
                "file_path": row.file_path,
                "scope": row.scope,
                "tags": row.tags,
                "score": score
            })
        })
        .collect();

    let acct = Accountant::from_conn(conn, engine.project_id(), engine.session_id());
    let ptr_tokens = (matches.len() as u64).saturating_mul(80) + 30;
    let _ = acct.record_query(
        &format!("match_skills:{query}"),
        ptr_tokens, 0,
        ptr_tokens.saturating_mul(15),
    );

    Ok(json!({ "matches": matches }))
}

/// Return full content and resource links for a skill.
pub fn tool_fetch_skill(engine: &crate::HermesEngine, skill_path: &str) -> Result<Value> {
    let db = engine.read_db().lock().unwrap_or_else(|e| e.into_inner());
    tool_fetch_skill_with_conn(engine, &db, skill_path)
}

pub fn tool_fetch_skill_with_conn(
    engine: &crate::HermesEngine,
    conn: &rusqlite::Connection,
    skill_path: &str,
) -> Result<Value> {
    let mut stmt = conn.prepare(
        "SELECT id, name, file_path, description, tags, version FROM skills \
         WHERE file_path = ?1 AND project_id = ?2",
    )?;
    let row = stmt.query_row(params![skill_path, engine.project_id()], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, Option<String>>(5)?,
        ))
    })?;

    let content = std::fs::read_to_string(skill_path)
        .map_err(|e| anyhow::anyhow!("failed to read skill file: {e}"))?;

    let acct = Accountant::from_conn(conn, engine.project_id(), engine.session_id());
    let tokens = (content.len() as u64) / 4;
    let _ = acct.record_query(&format!("fetch_skill:{skill_path}"), 0, tokens, tokens.saturating_mul(10));

    Ok(json!({
        "id": row.0,
        "name": row.1,
        "file_path": row.2,
        "description": row.3,
        "tags": row.4,
        "version": row.5,
        "content": content
    }))
}

struct SkillRow {
    id: String,
    name: String,
    description: String,
    category: String,
    language: String,
    version: String,
    file_path: String,
    scope: String,
    tags: String,
}

fn scope_precedence(scope: &str) -> u8 {
    match scope {
        "project" => 0,
        "shared" => 1,
        _ => 2,
    }
}
