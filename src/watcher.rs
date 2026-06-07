// tools/hermes-engine/src/watcher.rs
// TRACK-045: File-watcher module for incremental re-indexing using the `notify` crate.
//
// Replaces the timer-based auto-reindex loop when HERMES_USE_FILE_WATCHER=1.
// Falls back to timer mode (and logs a warning) if the watcher fails to start.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use notify::{
    Event, EventKind, RecommendedWatcher, RecursiveMode, Result as NotifyResult, Watcher,
};

use crate::ingestion::crawler;
use crate::mcp_actor::ToolActor;

const DEBOUNCE_SECS: u64 = 2;

/// Gitignore-style patterns to skip during watch events.
const IGNORE_SEGMENTS: &[&str] = &[
    ".git",
    ".hermes.db",
    "node_modules",
    "target",
    "build",
    "dist",
    ".gradle",
    ".next",
    ".turbo",
    "__pycache__",
    ".hermes-cache",
    "coverage",
    "out",
    "playwright-report",
    "test-results",
];

/// Start the file watcher in a background thread.
///
/// Returns `Ok(())` immediately — the watcher runs in a detached thread.
/// Panics or errors are caught and logged; the caller can fall back to timer mode.
pub fn start_file_watcher(actor: ToolActor, project_root: &Path) -> Result<(), String> {
    let project_root = project_root.to_path_buf();

    let (tx, rx) = mpsc::channel::<NotifyResult<Event>>();

    let mut watcher: RecommendedWatcher = notify::recommended_watcher(move |res| {
        let _ = tx.send(res);
    })
    .map_err(|e| format!("notify::recommended_watcher failed: {e}"))?;

    watcher
        .watch(&project_root, RecursiveMode::Recursive)
        .map_err(|e| format!("watcher.watch failed: {e}"))?;

    let display_path = project_root.display().to_string();
    std::thread::spawn(move || {
        // Keep watcher alive in this thread
        let _watcher = watcher;
        watch_loop(actor, rx, &project_root);
    });

    eprintln!("[hermes] file-watcher started on {display_path}");
    Ok(())
}

fn watch_loop(actor: ToolActor, rx: mpsc::Receiver<NotifyResult<Event>>, project_root: &Path) {
    let mut debounce_map: HashMap<PathBuf, Instant> = HashMap::new();
    let mut pending_reindex = false;
    let debounce = Duration::from_secs(DEBOUNCE_SECS);

    loop {
        // Receive with timeout so we can flush pending reindex after quiet period
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(Ok(event)) => {
                handle_event(
                    &event,
                    project_root,
                    &mut debounce_map,
                    &mut pending_reindex,
                    debounce,
                );
            }
            Ok(Err(e)) => {
                eprintln!("[hermes-watcher] notify error: {e}");
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if pending_reindex {
                    // Check if all debounced paths have quieted down
                    let now = Instant::now();
                    let all_quiet = debounce_map
                        .values()
                        .all(|t| now.duration_since(*t) >= debounce);
                    if all_quiet {
                        eprintln!("[hermes-watcher] triggering incremental re-index");
                        if let Err(e) = actor.enqueue_auto_index() {
                            eprintln!("[hermes-watcher] enqueue failed: {e}");
                        }
                        debounce_map.clear();
                        pending_reindex = false;
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                eprintln!("[hermes-watcher] channel disconnected, stopping");
                break;
            }
        }
    }
}

fn handle_event(
    event: &Event,
    project_root: &Path,
    debounce_map: &mut HashMap<PathBuf, Instant>,
    pending_reindex: &mut bool,
    debounce: Duration,
) {
    match event.kind {
        EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_) => {
            for path in &event.paths {
                if should_ignore(path, project_root) {
                    continue;
                }
                let last = debounce_map.entry(path.clone()).or_insert(Instant::now());
                *last = Instant::now();
                // Only schedule if not recently indexed
                if Instant::now().duration_since(*last) >= debounce || !*pending_reindex {
                    *pending_reindex = true;
                }
            }
        }
        _ => {}
    }
}

fn should_ignore(path: &Path, _project_root: &Path) -> bool {
    if path.is_dir() && crawler::should_ignore_dir(path) {
        return true;
    }
    let path_str = path.to_string_lossy();
    IGNORE_SEGMENTS.iter().any(|seg| path_str.contains(seg))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_ignore_git_dir() {
        let p = PathBuf::from("/project/.git/COMMIT_EDITMSG");
        assert!(should_ignore(&p, Path::new("/project")));
    }

    #[test]
    fn should_not_ignore_src_file() {
        let p = PathBuf::from("/project/src/main.rs");
        assert!(!should_ignore(&p, Path::new("/project")));
    }

    #[test]
    fn should_ignore_node_modules() {
        let p = PathBuf::from("/project/node_modules/foo/index.js");
        assert!(should_ignore(&p, Path::new("/project")));
    }

    #[test]
    fn should_ignore_android_build_output() {
        let p = PathBuf::from("/project/ChartApp/android-app/app/build/intermediates/merged.json");
        assert!(should_ignore(&p, Path::new("/project")));
    }

    #[test]
    fn should_ignore_python_cache_output() {
        let p = PathBuf::from("/project/service/__pycache__/worker.cpython-312.pyc");
        assert!(should_ignore(&p, Path::new("/project")));
    }
}
