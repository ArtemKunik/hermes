use chrono::{DateTime, Utc};
use regex::Regex;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;

pub(crate) fn latest_modified_at(dir: &Path) -> Option<String> {
    fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| entry.metadata().ok()?.modified().ok())
        .max()
        .map(|time| DateTime::<Utc>::from(time).to_rfc3339())
}

pub(crate) fn find_memory_hits(
    root: &Path,
    needle: &str,
    max_hits: usize,
) -> anyhow::Result<Vec<String>> {
    let mut hits = Vec::new();
    collect_hits(root, needle, max_hits, &mut hits)?;
    Ok(hits)
}

fn collect_hits(
    root: &Path,
    needle: &str,
    max_hits: usize,
    hits: &mut Vec<String>,
) -> anyhow::Result<()> {
    if !root.exists() || hits.len() >= max_hits {
        return Ok(());
    }
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect_hits(&path, needle, max_hits, hits)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("md")
            && fs::read_to_string(&path)
                .map(|text| text.contains(needle))
                .unwrap_or(false)
        {
            hits.push(path.to_string_lossy().replace('/', "\\"));
            if hits.len() >= max_hits {
                break;
            }
        }
    }
    Ok(())
}

pub(crate) fn git_track_commits(project_root: &Path, track_id: &str) -> anyhow::Result<Vec<String>> {
    let output = Command::new("git")
        .args(["--no-pager", "log", "--oneline", "--grep", track_id, "-n", "5"])
        .current_dir(project_root)
        .output()?;
    
    if !output.status.success() {
        return Ok(Vec::new());
    }

    let commits = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(String::from)
        .collect();
    
    Ok(commits)
}

pub(crate) fn extract_items(text: &str, prefix: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            trimmed.starts_with(prefix).then(|| clean_item(trimmed, prefix))
        })
        .filter(|line| !line.is_empty())
        .take(6)
        .collect()
}

pub(crate) fn extract_paths(text: &str) -> Vec<String> {
    let re = Regex::new(r"`([^`\n]+(?:/|\\)[^`\n]+)`").expect("valid path regex");
    let mut paths = BTreeSet::new();
    for cap in re.captures_iter(text) {
        let candidate = cap[1].replace('/', "\\");
        if is_likely_repo_path(&candidate) {
            paths.insert(candidate);
        }
    }
    paths.into_iter().take(10).collect()
}

pub(crate) fn compute_completion_pct(done: usize, remaining: usize) -> u8 {
    let total = done + remaining;
    if total > 0 {
        ((done * 100) / total) as u8
    } else {
        0
    }
}

fn clean_item(line: &str, prefix: &str) -> String {
    line.strip_prefix(prefix)
        .unwrap_or(line)
        .replace("`", "")
        .trim()
        .to_string()
}

fn is_likely_repo_path(candidate: &str) -> bool {
    if candidate.contains(' ') || candidate.contains(':') {
        return false;
    }

    let mut segments = candidate.split('\\');
    let Some(file_name) = segments.next_back() else {
        return false;
    };

    if !file_name.contains('.') || !is_safe_segment(file_name) {
        return false;
    }

    segments.all(is_safe_segment)
}

fn is_safe_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
}
