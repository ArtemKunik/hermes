use crate::mcp_tracks_support::{
    compute_completion_pct, extract_items, extract_paths, find_memory_hits, git_track_commits,
    latest_modified_at,
};
use crate::HermesEngine;
use anyhow::{anyhow, Result};
use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::path::Path;

#[derive(Clone, Debug, Serialize)]
pub struct TrackSummary {
    pub track_id: String,
    pub title: String,
    pub status: String,
    pub stale_docs: bool,
    pub completion_pct: u8,
    pub next_step: Option<String>,
    pub related_files: Vec<String>,
    pub updated_at: Option<String>,
    pub status_detail: String,
}

#[derive(Clone, Debug, Serialize)]
struct ResumeBrief {
    track_id: String,
    title: String,
    status: String,
    selection_reason: String,
    stale_docs: bool,
    status_detail: String,
    done: Vec<String>,
    remaining: Vec<String>,
    next_step: Option<String>,
    related_files: Vec<String>,
    related_sessions: Vec<String>,
    related_decisions: Vec<String>,
    recent_commits: Vec<String>,
    suggested_branch: String,
    continuation_prompt: String,
}

#[derive(Clone, Debug)]
pub(crate) struct TrackDocSet {
    pub(crate) track_id: String,
    pub(crate) title: String,
    pub(crate) status: String,
    pub(crate) stale_docs: bool,
    pub(crate) completion_pct: u8,
    pub(crate) next_step: Option<String>,
    pub(crate) related_files: Vec<String>,
    pub(crate) updated_at: Option<String>,
    pub(crate) status_detail: String,
    pub(crate) done: Vec<String>,
    pub(crate) remaining: Vec<String>,
}

pub fn tool_list_tracks(
    engine: &HermesEngine,
    project_root: &Path,
    args: &Value,
) -> Result<String> {
    let db = engine.read_db().lock().unwrap_or_else(|e| e.into_inner());
    tool_list_tracks_with_conn(engine, &db, project_root, args)
}

pub fn tool_list_tracks_with_conn(
    _engine: &HermesEngine,
    _conn: &rusqlite::Connection,
    project_root: &Path,
    args: &Value,
) -> Result<String> {
    let filter = args["status"].as_str().unwrap_or("unfinished");
    let tracks = load_tracks(project_root)?;
    let filtered: Vec<TrackSummary> = tracks
        .into_iter()
        .filter(|track| matches_filter(&track.status, filter))
        .map(|track| TrackSummary {
            track_id: track.track_id,
            title: track.title,
            status: track.status,
            stale_docs: track.stale_docs,
            completion_pct: track.completion_pct,
            next_step: track.next_step,
            related_files: track.related_files,
            updated_at: track.updated_at,
            status_detail: track.status_detail,
        })
        .collect();
    Ok(serde_json::to_string_pretty(&filtered)?)
}

pub fn tool_resume_track(
    engine: &HermesEngine,
    project_root: &Path,
    args: &Value,
) -> Result<String> {
    let db = engine.read_db().lock().unwrap_or_else(|e| e.into_inner());
    tool_resume_track_with_conn(engine, &db, project_root, args)
}

pub fn tool_resume_track_with_conn(
    _engine: &HermesEngine,
    _conn: &rusqlite::Connection,
    project_root: &Path,
    args: &Value,
) -> Result<String> {
    let tracks = load_tracks(project_root)?;
    let requested_id = args["track_id"]
        .as_str()
        .map(str::trim)
        .filter(|id| !id.is_empty());
    let filter = args["status"].as_str().unwrap_or("unfinished");
    let selected = if let Some(track_id) = requested_id {
        tracks
            .iter()
            .find(|track| track.track_id == track_id)
            .ok_or_else(|| anyhow!("track not found: {track_id}"))?
            .clone()
    } else if args["auto"].as_bool().unwrap_or(false) {
        pick_best_track(&tracks, filter)?
    } else {
        return Err(anyhow!(
            "hermes_resume_track requires 'track_id' or auto=true"
        ));
    };

    if selected.status == "conflict" {
        return Err(anyhow!(
            "conflicting track docs for {} — resolve status mismatch before resuming",
            selected.track_id
        ));
    }

    let reason = if requested_id.is_some() {
        "explicitly requested".to_string()
    } else {
        format!(
            "auto-selected (score={}, status={})",
            rank_track(&selected),
            selected.status
        )
    };

    let hits = find_memory_hits(project_root, &selected.track_id, 20)?;
    let sessions = hits
        .iter()
        .filter(|h| h.contains("sessions"))
        .cloned()
        .collect();
    let decisions = hits
        .iter()
        .filter(|h| h.contains("decisions"))
        .cloned()
        .collect();
    let commits = git_track_commits(project_root, &selected.track_id)?;

    let continuation_prompt = format!(
        "I'm resuming work on {}: {}. My current status is {}. {} remaining tasks. \
         The immediate next step is: {}",
        selected.track_id,
        selected.title,
        selected.status,
        selected.remaining.len(),
        selected
            .next_step
            .as_deref()
            .unwrap_or("continue investigation")
    );

    let brief = ResumeBrief {
        track_id: selected.track_id.clone(),
        title: selected.title.clone(),
        status: selected.status.clone(),
        selection_reason: reason,
        stale_docs: selected.stale_docs,
        status_detail: selected.status_detail,
        done: selected.done,
        remaining: selected.remaining,
        next_step: selected.next_step,
        related_files: selected.related_files,
        related_sessions: sessions,
        related_decisions: decisions,
        recent_commits: commits,
        suggested_branch: format!("track/{}", selected.track_id.to_lowercase()),
        continuation_prompt,
    };

    Ok(serde_json::to_string_pretty(&brief)?)
}

pub(crate) fn load_tracks(root: &Path) -> Result<Vec<TrackDocSet>> {
    let mut tracks = Vec::new();
    let tracks_dir = root.join("conductor").join("tracks");
    if !tracks_dir.exists() {
        return Ok(tracks);
    }

    for entry in fs::read_dir(tracks_dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            let track_id = entry.file_name().to_string_lossy().to_string();
            if track_id.starts_with("TRACK-") {
                if let Ok(doc_set) = parse_track_dir(&entry.path(), &track_id) {
                    tracks.push(doc_set);
                }
            }
        }
    }
    tracks.sort_by(|a, b| a.track_id.cmp(&b.track_id));
    Ok(tracks)
}

fn parse_track_dir(path: &Path, track_id: &str) -> Result<TrackDocSet> {
    let index_md = fs::read_to_string(path.join("index.md"))?;
    let plan_md = fs::read_to_string(path.join("plan.md")).unwrap_or_default();

    let title = index_md
        .lines()
        .find_map(|line| line.strip_prefix("# "))
        .map(|s| s.replace(track_id, "").replace(':', "").trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Untitled Track".to_string());

    let index_status = extract_status(&index_md);
    let plan_status = extract_status(&plan_md);
    let status = merge_status(index_status.as_deref(), plan_status.as_deref());
    let updated_at = extract_items(&index_md, "**Updated**: ").first().cloned();
    let related_files = {
        let mut paths = extract_paths(&index_md);
        paths.extend(extract_paths(&plan_md));
        paths.sort();
        paths.dedup();
        paths
    };

    let done = extract_items(&plan_md, "- [x] ");
    let remaining = extract_items(&plan_md, "- [ ] ");
    let completion_pct = compute_completion_pct(done.len(), remaining.len());
    let next_step = remaining.first().cloned();

    let mtime = latest_modified_at(path);
    let stale_docs = status == "conflict"
        || updated_at
            .as_ref()
            .map(|u| mtime.as_deref().unwrap_or_default() > u.as_str())
            .unwrap_or(false);

    Ok(TrackDocSet {
        track_id: track_id.to_string(),
        title,
        status,
        stale_docs,
        completion_pct,
        next_step,
        related_files,
        updated_at: mtime,
        status_detail: format!(
            "{}/{} tasks complete",
            done.len(),
            done.len() + remaining.len()
        ),
        done,
        remaining,
    })
}

fn extract_status(doc: &str) -> Option<String> {
    let lines: Vec<&str> = doc.lines().collect();
    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("## Status:") {
            return normalize_status(value);
        }
        if let Some(value) = trimmed.strip_prefix("**Status**:") {
            return normalize_status(value);
        }
        if trimmed.eq_ignore_ascii_case("## Status") {
            let next = lines
                .iter()
                .skip(idx + 1)
                .map(|line| line.trim())
                .find(|line| !line.is_empty() && !line.starts_with('#'))?;
            return normalize_status(next);
        }
    }
    None
}

fn normalize_status(value: &str) -> Option<String> {
    let normalized = value.trim().trim_matches('*').trim().to_ascii_lowercase();
    let mapped = match normalized.as_str() {
        "active" | "in progress" | "in-progress" => "in-progress",
        "planned" | "planning" => "planned",
        "speccing" | "spec" => "speccing",
        "complete" | "completed" | "done" => "completed",
        "blocked" => "blocked",
        "cancelled" | "canceled" => "cancelled",
        "unknown" | "" => return None,
        other => other,
    };
    Some(mapped.to_string())
}

fn merge_status(index_status: Option<&str>, plan_status: Option<&str>) -> String {
    match (index_status, plan_status) {
        (Some(left), Some(right)) if left != right => "conflict".to_string(),
        (Some(status), _) | (_, Some(status)) => status.to_string(),
        (None, None) => "unknown".to_string(),
    }
}

pub(crate) fn pick_best_track(tracks: &[TrackDocSet], filter: &str) -> Result<TrackDocSet> {
    tracks
        .iter()
        .filter(|t| matches_filter(&t.status, filter))
        .max_by_key(|track| rank_track(track))
        .cloned()
        .ok_or_else(|| anyhow!("no matching tracks found to auto-resume"))
}

fn rank_track(track: &TrackDocSet) -> i64 {
    let status_bonus = if track.status == "in-progress" {
        1000
    } else {
        0
    };
    let action_bonus = if track.next_step.is_some() || !track.remaining.is_empty() {
        200
    } else {
        -1200
    };
    status_bonus + action_bonus - track.completion_pct as i64
}

fn matches_filter(status: &str, filter: &str) -> bool {
    match filter {
        "unfinished" => status != "completed" && status != "cancelled" && status != "conflict",
        "active" => status == "in-progress",
        "all" => true,
        other => status == other,
    }
}
