// ChartApp/hermes-engine/src/ingestion/crawler.rs
use anyhow::Result;
use std::path::{Path, PathBuf};

const SUPPORTED_EXTENSIONS: &[&str] = &[
    // Rust
    "rs", // Web — TypeScript / JavaScript / CSS
    "tsx", "ts", "jsx", "js", "css", // Markup / config
    "md", "toml", "json", "yml", "yaml", // Python
    "py",   // Shell / scripting
    "sh", "ps1", // Infrastructure
    "tf",
];

const IGNORED_DIRS: &[&str] = &[
    // Rust build artefacts — any variant of "target" that appears as a build dir
    "target",
    "target-alt",
    "target-scratch",
    "target-test",
    "target-clean",
    "target-track053",
    // JS/Python tooling dirs
    "node_modules",
    ".venv",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    "dist",
    "build",
    ".gradle",
    ".next",
    ".turbo",
    ".vite",
    "__pycache__",
    "coverage",
    "out",
    // Git internals
    ".git",
    // Git worktrees — each contains a full project copy; indexing them would
    // duplicate every symbol and make searches non-deterministic.
    ".codex-worktrees",
    ".copilot-worktrees",
    // Swarm orchestrator working dirs (per-run JSON state, not source)
    ".swarm",
    // Test/CI output directories
    "output",
    "playwright-report",
    "test-results",
    // Research pipeline artifacts & data files — natural language content
    // inside these pollutes vector search results with non-code matches.
    "artifacts",
    "sessions",
    "rejected",
    "daemon_logs",
    "mastermind-logs",
    "mastermind-logs-native",
    "mastermind-logs-new",
    // Root-level data directory contains JSON control-plane state, not source
    "data",
    // Sandbox experiments are ephemeral and not part of the indexed codebase
    "sandbox",
];

/// Directories that are always crawled even if their name matches IGNORED_DIRS.
/// This handles cases like `memory/sessions/` which must be indexed for Hermes
/// Memory recall, even though `sessions` is otherwise ignored.
const ALLOWED_PARENT_DIRS: &[&str] = &["memory"];

pub fn crawl_directory(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    crawl_recursive(dir, &mut files)?;
    files.sort();
    Ok(files)
}

fn crawl_recursive(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }

    if should_ignore_dir(dir) {
        return Ok(());
    }

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            crawl_recursive(&path, files)?;
        } else if is_supported_file(&path) {
            files.push(path);
        }
    }

    Ok(())
}

pub fn should_ignore_dir(dir: &Path) -> bool {
    let dir_name = dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    IGNORED_DIRS.contains(&dir_name.as_str()) && !has_allowed_ancestor(dir)
}

fn is_supported_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| SUPPORTED_EXTENSIONS.contains(&ext))
        .unwrap_or(false)
}

/// Returns true when any ancestor directory of `dir` has a name listed in
/// ALLOWED_PARENT_DIRS. This lets `memory/sessions/` be indexed even though
/// `sessions` is in IGNORED_DIRS.
fn has_allowed_ancestor(dir: &Path) -> bool {
    let mut current = dir.parent();
    while let Some(p) = current {
        if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
            if ALLOWED_PARENT_DIRS.contains(&name) {
                return true;
            }
        }
        current = p.parent();
    }
    false
}

/// Returns true when the given path is inside the `memory/` directory tree.
/// Used by search to classify results as memory vs code.
pub fn is_memory_path(path: &str) -> bool {
    // Normalise separators for cross-platform matching
    let normalised = path.replace('\\', "/");
    normalised.contains("/memory/sessions/")
        || normalised.contains("/memory/decisions/")
        || normalised.contains("/memory/handover/")
}

/// Returns true when the path is inside `memory/decisions/`.
pub fn is_decision_path(path: &str) -> bool {
    path.replace('\\', "/").contains("/memory/decisions/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn crawl_finds_rust_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();
        fs::write(dir.path().join("readme.txt"), "not indexed").unwrap();

        let files = crawl_directory(dir.path()).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].to_string_lossy().contains("main.rs"));
    }

    #[test]
    fn crawl_ignores_node_modules() {
        let dir = tempfile::tempdir().unwrap();
        let nm = dir.path().join("node_modules");
        fs::create_dir(&nm).unwrap();
        fs::write(nm.join("lib.js"), "module.exports = {}").unwrap();
        fs::write(dir.path().join("app.ts"), "const x = 1;").unwrap();

        let files = crawl_directory(dir.path()).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].to_string_lossy().contains("app.ts"));
    }

    #[test]
    fn crawl_ignores_build_dir() {
        let dir = tempfile::tempdir().unwrap();
        let build = dir.path().join("build");
        fs::create_dir(&build).unwrap();
        fs::write(build.join("generated.json"), r#"{"generated":true}"#).unwrap();
        fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();

        let files = crawl_directory(dir.path()).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].to_string_lossy().contains("main.rs"));
    }

    #[test]
    fn crawl_ignores_python_cache_dir() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("__pycache__");
        fs::create_dir(&cache).unwrap();
        fs::write(cache.join("module.pyc"), "compiled").unwrap();
        fs::write(dir.path().join("worker.py"), "print('ok')").unwrap();

        let files = crawl_directory(dir.path()).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].to_string_lossy().contains("worker.py"));
    }

    #[test]
    fn crawl_ignores_artifacts_dir() {
        let dir = tempfile::tempdir().unwrap();
        let arts = dir.path().join("artifacts");
        fs::create_dir(&arts).unwrap();
        fs::write(arts.join("synthesis.json"), r#"{"content":"for loops"}"#).unwrap();
        fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();

        let files = crawl_directory(dir.path()).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].to_string_lossy().contains("main.rs"));
    }

    #[test]
    fn crawl_ignores_data_dir() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        fs::create_dir(&data).unwrap();
        fs::write(data.join("state.json"), r#"{"op":"control"}"#).unwrap();
        fs::write(dir.path().join("lib.ts"), "export const x = 1;").unwrap();

        let files = crawl_directory(dir.path()).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].to_string_lossy().contains("lib.ts"));
    }

    #[test]
    fn crawl_ignores_daemon_logs_dir() {
        let dir = tempfile::tempdir().unwrap();
        let logs = dir.path().join("daemon_logs");
        fs::create_dir(&logs).unwrap();
        fs::write(logs.join("app.json"), r#"{"log":"entry"}"#).unwrap();
        fs::write(dir.path().join("service.rs"), "pub fn run() {}").unwrap();

        let files = crawl_directory(dir.path()).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].to_string_lossy().contains("service.rs"));
    }

    #[test]
    fn supported_extensions_check() {
        assert!(is_supported_file(Path::new("foo.rs")));
        assert!(is_supported_file(Path::new("bar.tsx")));
        assert!(is_supported_file(Path::new("doc.md")));
        assert!(is_supported_file(Path::new("pipeline.yml")));
        assert!(is_supported_file(Path::new("config.yaml")));
        assert!(is_supported_file(Path::new("agent.py")));
        assert!(is_supported_file(Path::new("deploy.sh")));
        assert!(is_supported_file(Path::new("build.ps1")));
        assert!(is_supported_file(Path::new("main.tf")));
        assert!(!is_supported_file(Path::new("image.png")));
        assert!(!is_supported_file(Path::new("data.csv")));
        assert!(!is_supported_file(Path::new("binary.exe")));
    }

    #[test]
    fn crawl_includes_memory_sessions_despite_sessions_ignore() {
        let dir = tempfile::tempdir().unwrap();
        let mem = dir.path().join("memory").join("sessions");
        fs::create_dir_all(&mem).unwrap();
        fs::write(mem.join("2026-02-27_test.md"), "# Session: test").unwrap();

        // Also write a top-level sessions/ dir that should still be ignored
        let top_sessions = dir.path().join("sessions");
        fs::create_dir(&top_sessions).unwrap();
        fs::write(top_sessions.join("noise.md"), "# Ignored").unwrap();

        let files = crawl_directory(dir.path()).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].to_string_lossy().contains("memory"));
    }

    #[test]
    fn is_memory_path_detects_sessions() {
        assert!(is_memory_path(
            "/project/memory/sessions/2026-02-27_test.md"
        ));
        assert!(is_memory_path("D:\\project\\memory\\sessions\\test.md"));
        assert!(is_memory_path("/project/memory/decisions/ADR-001.md"));
        assert!(!is_memory_path("/project/src/main.rs"));
        assert!(!is_memory_path("/project/sessions/noise.md"));
    }

    #[test]
    fn should_ignore_dir_allows_memory_sessions_but_blocks_build() {
        let dir = tempfile::tempdir().unwrap();
        let build = dir.path().join("build");
        let memory_sessions = dir.path().join("memory").join("sessions");
        fs::create_dir_all(&build).unwrap();
        fs::create_dir_all(&memory_sessions).unwrap();

        assert!(should_ignore_dir(&build));
        assert!(!should_ignore_dir(&memory_sessions));
    }
}
