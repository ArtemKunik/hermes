use crate::mcp_tracks::{load_tracks, pick_best_track};
use std::fs;
use tempfile::tempdir;

#[test]
fn list_tracks_loads_basic_metadata() {
    let dir = tempdir().unwrap();
    let track_dir = dir
        .path()
        .join("conductor")
        .join("tracks")
        .join("TRACK-999");
    fs::create_dir_all(&track_dir).unwrap();
    fs::write(
        track_dir.join("index.md"),
        "# TRACK-999: Test Track\n\n**Status**: completed\n",
    )
    .unwrap();
    fs::write(track_dir.join("plan.md"), "- [x] task 1\n- [ ] task 2\n").unwrap();

    let tracks = load_tracks(dir.path()).unwrap();
    let track = tracks.first().unwrap();
    assert_eq!(track.track_id, "TRACK-999");
    assert_eq!(track.title, "Test Track");
    assert_eq!(track.status, "completed");
    assert_eq!(track.completion_pct, 50);
}

#[test]
fn resume_prefers_inprogress_track_in_auto_mode() {
    let dir = tempdir().unwrap();
    for (name, status) in [("TRACK-100", "planned"), ("TRACK-101", "in-progress")] {
        let track_dir = dir.path().join("conductor").join("tracks").join(name);
        fs::create_dir_all(&track_dir).unwrap();
        fs::write(
            track_dir.join("index.md"),
            format!("# {name}\n\n**Status**: {status}\n"),
        )
        .unwrap();
        fs::write(track_dir.join("plan.md"), "- [ ] first task\n").unwrap();
    }
    let chosen = pick_best_track(&load_tracks(dir.path()).unwrap(), "unfinished").unwrap();
    assert_eq!(chosen.track_id, "TRACK-101");
}

#[test]
fn load_tracks_filters_prose_from_related_files() {
    let dir = tempdir().unwrap();
    let track_dir = dir
        .path()
        .join("conductor")
        .join("tracks")
        .join("TRACK-102");
    fs::create_dir_all(&track_dir).unwrap();
    fs::write(
        track_dir.join("index.md"),
        "# TRACK-102\n\n## Status: active\n\nRead `conductor/tech-stack.md` first.\nIgnore `command) with file context, SEARCH\\\\REPLACE patches, and `.\n",
    )
    .unwrap();
    fs::write(track_dir.join("plan.md"), "- [ ] next task\n").unwrap();

    let track = load_tracks(dir.path()).unwrap().pop().unwrap();
    assert_eq!(track.related_files, vec!["conductor\\tech-stack.md"]);
}

#[test]
fn resume_prefers_actionable_track_over_finished_inprogress_track() {
    let dir = tempdir().unwrap();
    for (name, status, plan) in [
        ("TRACK-200", "in-progress", "- [x] wrapped up\n"),
        ("TRACK-201", "planned", "- [ ] next task\n"),
    ] {
        let track_dir = dir.path().join("conductor").join("tracks").join(name);
        fs::create_dir_all(&track_dir).unwrap();
        fs::write(
            track_dir.join("index.md"),
            format!("# {name}\n\n**Status**: {status}\n"),
        )
        .unwrap();
        fs::write(track_dir.join("plan.md"), plan).unwrap();
    }

    let chosen = pick_best_track(&load_tracks(dir.path()).unwrap(), "unfinished").unwrap();
    assert_eq!(chosen.track_id, "TRACK-201");
}
