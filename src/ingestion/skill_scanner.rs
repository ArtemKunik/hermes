use anyhow::Result;
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};
use tracing::debug;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillMetadata {
    pub name: String,
    pub description: String,
    pub category: String,
    pub language: String,
    pub version: String,
    pub file_path: String,
    pub scope: String,
    pub tags: String,
}

#[derive(Default)]
struct ParsedSkillFields {
    name: Option<String>,
    description: Option<String>,
    category: Option<String>,
    language: Option<String>,
    version: Option<String>,
    tags: Vec<String>,
}

pub fn discover_skills(project_root: &Path) -> Vec<SkillMetadata> {
    let skills_dir = project_root.join("skills");
    if !skills_dir.is_dir() {
        return Vec::new();
    }

    let mut files = Vec::new();
    collect_skill_files(&skills_dir, &mut files);
    files.sort();
    files
        .into_iter()
        .filter_map(|path| parse_skill_file(&path))
        .collect()
}

fn collect_skill_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_skill_files(&path, out);
        } else if is_skill_file(&path) {
            out.push(path);
        }
    }
}

fn is_skill_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.eq_ignore_ascii_case("SKILL.md"))
        .unwrap_or(false)
}

pub fn parse_skill_file(path: &Path) -> Option<SkillMetadata> {
    let content = std::fs::read_to_string(path).ok()?;
    let parsed = parse_skill_content(&content);
    let (language, category) = infer_path_metadata(path);
    let file_path = path.to_string_lossy().to_string();

    Some(SkillMetadata {
        name: parsed
            .name
            .unwrap_or_else(|| fallback_skill_name(path)),
        description: parsed.description.unwrap_or_default(),
        category: parsed.category.unwrap_or(category),
        language: parsed.language.unwrap_or(language),
        version: parsed.version.unwrap_or_default(),
        file_path: file_path.clone(),
        scope: determine_scope(&file_path),
        tags: normalize_tags(&parsed.tags),
    })
}

fn parse_skill_content(content: &str) -> ParsedSkillFields {
    let (frontmatter, body) = split_frontmatter(content);
    let mut parsed = ParsedSkillFields::default();

    if let Some(frontmatter) = frontmatter {
        parse_metadata_block(&frontmatter, &mut parsed);
    }
    parse_body(&body, &mut parsed);

    if parsed.description.is_none() {
        parsed.description = first_body_paragraph(&body);
    }
    parsed
}

fn split_frontmatter(content: &str) -> (Option<String>, String) {
    let content = content.trim_start_matches('\u{feff}');
    let lines: Vec<&str> = content.lines().collect();
    if lines.first().map(|line| line.trim()) != Some("---") {
        return (None, content.to_string());
    }

    let Some(end_idx) = lines.iter().enumerate().skip(1).find_map(|(idx, line)| {
        (line.trim() == "---").then_some(idx)
    }) else {
        return (None, content.to_string());
    };

    let frontmatter = lines[1..end_idx].join("\n");
    let body = lines[end_idx + 1..].join("\n");
    (Some(frontmatter), body)
}

fn parse_body(body: &str, parsed: &mut ParsedSkillFields) {
    let mut section = String::new();

    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed == "```" {
            continue;
        }

        if trimmed.starts_with("## ") {
            section = trimmed.trim_start_matches('#').trim().to_lowercase();
            continue;
        }

        if parsed.name.is_none() && trimmed.starts_with("# ") {
            parsed.name = Some(clean_skill_name(trimmed.trim_start_matches('#').trim()));
            continue;
        }

        if let Some((key, value)) = extract_metadata_line(trimmed) {
            apply_metadata_field(parsed, &key, &value);
            continue;
        }

        if parsed.description.is_none() && section == "purpose" {
            parsed.description = Some(trimmed.to_string());
        }
    }
}

fn parse_metadata_block(block: &str, parsed: &mut ParsedSkillFields) {
    let mut list_key: Option<String> = None;

    for line in block.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Some(item) = parse_list_item(trimmed) {
            if list_key.as_deref() == Some("tags") {
                parsed.tags.push(item);
            }
            continue;
        }

        list_key = None;
        if let Some((key, value)) = extract_metadata_line(trimmed) {
            let key = key.to_lowercase();
            if key == "tags" && value.is_empty() {
                list_key = Some(key);
                continue;
            }
            apply_metadata_field(parsed, &key, &value);
        }
    }
}

fn apply_metadata_field(parsed: &mut ParsedSkillFields, key: &str, value: &str) {
    let value = value.trim();
    match key.to_lowercase().as_str() {
        "name" | "skill_name" if !value.is_empty() => parsed.name = Some(value.to_string()),
        "description" if !value.is_empty() => parsed.description = Some(value.to_string()),
        "category" if !value.is_empty() => parsed.category = Some(value.to_lowercase()),
        "language" if !value.is_empty() => parsed.language = Some(value.to_lowercase()),
        "version" if !value.is_empty() => parsed.version = Some(value.to_string()),
        "tags" if !value.is_empty() => parsed.tags.extend(split_inline_tags(value)),
        _ => {}
    }
}

fn extract_metadata_line(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim_start_matches('-').trim();

    if trimmed.starts_with("**") {
        let rest = trimmed.trim_start_matches("**");
        let idx = rest.find("**")?;
        let key = rest[..idx].trim();
        let value = rest[idx + 2..].trim_start_matches(':').trim();
        return Some((key.to_string(), value.to_string()));
    }

    let idx = trimmed.find(':')?;
    let key = trimmed[..idx].trim();
    if key.is_empty() {
        return None;
    }
    Some((key.to_string(), trimmed[idx + 1..].trim().to_string()))
}

fn parse_list_item(line: &str) -> Option<String> {
    line.strip_prefix("- ")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.trim_matches('"').trim_matches('\'').to_string())
}

fn split_inline_tags(value: &str) -> Vec<String> {
    value
        .trim_matches('[')
        .trim_matches(']')
        .split(',')
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(|tag| tag.trim_matches('"').trim_matches('\'').to_string())
        .collect()
}

fn clean_skill_name(name: &str) -> String {
    name.trim_start_matches("Skill:")
        .trim_start_matches("skill:")
        .trim()
        .to_string()
}

fn first_body_paragraph(body: &str) -> Option<String> {
    body.lines()
        .map(str::trim)
        .find(|line| {
            !line.is_empty()
                && !line.starts_with('#')
                && !line.starts_with("```")
                && extract_metadata_line(line).is_none()
        })
        .map(str::to_string)
}

fn fallback_skill_name(path: &Path) -> String {
    path.parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("unnamed-skill")
        .to_string()
}

fn infer_path_metadata(path: &Path) -> (String, String) {
    let parts: Vec<String> = path
        .parent()
        .into_iter()
        .flat_map(|parent| parent.iter())
        .filter_map(|part| part.to_str().map(|value| value.to_lowercase()))
        .collect();

    let Some(skills_idx) = parts.iter().position(|part| part == "skills") else {
        return (String::new(), String::new());
    };

    let Some(language) = parts.get(skills_idx + 1).cloned() else {
        return (String::new(), String::new());
    };
    let category = parts.get(skills_idx + 2).cloned().unwrap_or_default();

    if is_language_dir(&language) {
        (language, category)
    } else {
        (String::new(), String::new())
    }
}

fn is_language_dir(part: &str) -> bool {
    matches!(
        part,
        "rust" | "python" | "react" | "powershell" | "cross-platform"
    )
}

fn determine_scope(file_path: &str) -> String {
    let normalized = file_path.replace('\\', "/");
    if normalized.contains("/.copilot/skills/") || normalized.contains("/.codex/skills/") {
        "shared".to_string()
    } else if normalized.contains("/skills/") {
        "project".to_string()
    } else {
        "global".to_string()
    }
}

fn normalize_tags(tags: &[String]) -> String {
    let mut ordered = Vec::new();
    for tag in tags {
        if !ordered.contains(tag) {
            ordered.push(tag.clone());
        }
    }
    ordered.join(",")
}

pub fn skill_id(project_id: &str, file_path: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    project_id.hash(&mut hasher);
    file_path.hash(&mut hasher);
    format!("sk-{:016x}", hasher.finish())
}

pub fn populate_skills(conn: &Connection, project_id: &str, skills: &[SkillMetadata]) -> Result<()> {
    conn.execute("BEGIN", [])?;
    conn.execute("DELETE FROM skills WHERE project_id = ?1", params![project_id])?;

    for skill in skills {
        let id = skill_id(project_id, &skill.file_path);
        conn.execute(
            "INSERT INTO skills \
             (id, project_id, name, description, category, language, version, file_path, scope, tags, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, datetime('now'))",
            params![
                id,
                project_id,
                skill.name,
                skill.description,
                skill.category,
                skill.language,
                skill.version,
                skill.file_path,
                skill.scope,
                skill.tags
            ],
        )?;
        debug!(name = %skill.name, path = %skill.file_path, "Upserted skill");
    }

    conn.execute("COMMIT", [])?;
    Ok(())
}
