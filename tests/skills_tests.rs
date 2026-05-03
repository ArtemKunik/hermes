use hermes_engine::{
    ingestion::skill_scanner,
    mcp_skills,
    mcp_tools,
    schema,
    HermesEngine,
};
use rusqlite::Connection;
use serde_json::Value;
use std::fs;

#[test]
// COHERENCE: Integration fixture validates canonical SKILL.md frontmatter parsing and path metadata inference in one focused end-to-end test.
fn test_discover_skills_parses_frontmatter_and_infers_path_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let skill_dir = dir
        .path()
        .join("skills")
        .join("python")
        .join("automation")
        .join("submit-training-task");
    fs::create_dir_all(skill_dir.join("scripts")).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        r#"---
name: Submit Training Task
description: Submit training jobs to the Training API.
version: 1.2.0
tags:
  - training
  - automation
---

# Submit Training Task

Use this skill when you need to enqueue a training task.
"#,
    )
    .unwrap();

    let skills = skill_scanner::discover_skills(dir.path());
    assert_eq!(skills.len(), 1);

    let skill = &skills[0];
    assert_eq!(skill.name, "Submit Training Task");
    assert_eq!(skill.description, "Submit training jobs to the Training API.");
    assert_eq!(skill.language, "python");
    assert_eq!(skill.category, "automation");
    assert_eq!(skill.version, "1.2.0");
    assert_eq!(skill.scope, "project");
    assert_eq!(skill.tags, "training,automation");
}

#[test]
// COHERENCE: Integration fixture verifies stale skill rows are removed when the discovered set changes.
fn test_populate_skills_removes_stale_rows_for_missing_files() {
    let conn = Connection::open_in_memory().unwrap();
    schema::run_migrations(&conn).unwrap();

    conn.execute(
        "INSERT INTO skills (id, project_id, name, file_path, scope) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            "sk-old",
            "project-a",
            "Old Skill",
            "/repo/skills/rust/old-skill/SKILL.md",
            "project"
        ],
    )
    .unwrap();

    let skills = vec![skill_scanner::SkillMetadata {
        name: "New Skill".to_string(),
        description: "Freshly indexed".to_string(),
        category: "api".to_string(),
        language: "rust".to_string(),
        version: "1.0.0".to_string(),
        file_path: "/repo/skills/rust/api/new-skill/SKILL.md".to_string(),
        scope: "project".to_string(),
        tags: "rust,api".to_string(),
    }];

    skill_scanner::populate_skills(&conn, "project-a", &skills).unwrap();

    let mut stmt = conn
        .prepare("SELECT name FROM skills WHERE project_id = ?1 ORDER BY name")
        .unwrap();
    let names: Vec<String> = stmt
        .query_map(["project-a"], |row| row.get(0))
        .unwrap()
        .collect::<Result<Vec<String>, _>>()
        .unwrap();
    assert_eq!(names, vec!["New Skill".to_string()]);
}

#[test]
// COHERENCE: Integration fixture exercises indexed skill fetch plus resource-root discovery together because those behaviors fail in the same workflow.
fn test_index_and_fetch_skill_reports_resource_roots() {
    let dir = tempfile::tempdir().unwrap();
    let skill_dir = dir
        .path()
        .join("skills")
        .join("rust")
        .join("api")
        .join("http-request-builder");
    fs::create_dir_all(skill_dir.join("scripts")).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        r#"---
name: HTTP Request Builder
description: Build typed HTTP requests with retry-aware defaults.
tags:
  - http
  - reqwest
---

# HTTP Request Builder

Use this skill when a Rust service needs a reusable outbound HTTP pattern.
"#,
    )
    .unwrap();
    fs::write(
        skill_dir.join("scripts").join("builder.rs.txt"),
        "fn build_request() {}",
    )
    .unwrap();

    let engine = HermesEngine::in_memory("skills-index").unwrap();
    mcp_tools::tool_index(&engine, dir.path()).unwrap();

    let matched = mcp_skills::tool_match_skills(&engine, "http request retry", None).unwrap();
    let first_match = matched["matches"].as_array().unwrap()[0].clone();
    assert_eq!(first_match["name"], "HTTP Request Builder");
    assert_eq!(first_match["category"], "api");
    assert_eq!(first_match["language"], "rust");

    let fetched = mcp_skills::tool_fetch_skill(
        &engine,
        first_match["file_path"].as_str().unwrap(),
    )
    .unwrap();
    let resource_roots = fetched["resource_roots"].as_array().unwrap();
    assert_eq!(resource_roots, &[Value::String("scripts".to_string())]);
}

#[test]
// COHERENCE: Integration fixture verifies a second index pass drops removed canonical skills from the live table.
fn test_reindex_removes_skills_that_no_longer_exist_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let skill_dir = dir
        .path()
        .join("skills")
        .join("rust")
        .join("testing")
        .join("unit-test-generator");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        r#"---
name: Unit Test Generator
description: Generate Rust unit tests from function behavior.
---
"#,
    )
    .unwrap();

    let engine = HermesEngine::in_memory("skills-reindex").unwrap();
    mcp_tools::tool_index(&engine, dir.path()).unwrap();
    let first = mcp_skills::tool_match_skills(&engine, "unit test", None).unwrap();
    assert_eq!(first["total_matches"], 1);

    fs::remove_dir_all(dir.path().join("skills")).unwrap();

    mcp_tools::tool_index(&engine, dir.path()).unwrap();
    let second = mcp_skills::tool_match_skills(&engine, "unit test", None).unwrap();
    assert_eq!(second["total_matches"], 0);
}

#[test]
// COHERENCE: Integration fixture locks down relative skill-path lookup for Windows-style stored paths.
fn test_fetch_skill_accepts_relative_path_lookup() {
    let dir = tempfile::tempdir().unwrap();
    let skill_dir = dir
        .path()
        .join("skills")
        .join("python")
        .join("automation")
        .join("submit-training-task");
    fs::create_dir_all(skill_dir.join("scripts")).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        r#"---
name: Submit Training Task
description: Submit tasks through the Training API.
---
"#,
    )
    .unwrap();

    let engine = HermesEngine::in_memory("skills-relative-fetch").unwrap();
    mcp_tools::tool_index(&engine, dir.path()).unwrap();

    let fetched = mcp_skills::tool_fetch_skill(
        &engine,
        "skills/python/automation/submit-training-task/SKILL.md",
    )
    .unwrap();
    assert_eq!(fetched["name"], "Submit Training Task");
}
