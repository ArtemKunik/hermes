use anyhow::Result;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use tracing::{info, warn};

use crate::{
    graph::KnowledgeGraph,
    ingestion::IngestionPipeline,
    lock_ext::LockExt,
    HermesEngine,
};

// Registry entry: a pre-registered project path (from HERMES_PROJECTS env var).
// Stored as the canonical absolute path; project_id is its basename.
pub(crate) struct RegistryEntry {
    pub(crate) canonical: PathBuf,
    pub(crate) project_id: String,
}

// Caches engines keyed by canonicalized project root.
// A single MCP process can serve any number of repositories via:
//   1. HERMES_PROJECTS env var — pre-register paths at startup
//   2. Passing a project name (basename) or full path as project_root per call
pub(crate) struct EngineCache {
    pub(crate) default_engine: HermesEngine,
    pub(crate) default_root: PathBuf,
    extra: Mutex<HashMap<PathBuf, HermesEngine>>,
    // name → (canonical path, project_id) for HERMES_PROJECTS-registered projects
    registry: Vec<RegistryEntry>,
}

impl EngineCache {
    pub(crate) fn new(engine: HermesEngine, root: PathBuf, registry: Vec<RegistryEntry>) -> Self {
        Self {
            default_engine: engine,
            default_root: root,
            extra: Mutex::new(HashMap::new()),
            registry,
        }
    }

    // Resolve a project_root argument to (engine, canonical_path).
    // Accepts:
    //   - a project name / basename  ("lonaspark")
    //   - an absolute path           ("D:/source/lonaspark")
    // Registry entries (from HERMES_PROJECTS) are checked by name first.
    pub(crate) fn resolve(&self, project_root_arg: Option<&str>) -> Result<(HermesEngine, PathBuf)> {
        let Some(arg) = project_root_arg.filter(|s| !s.is_empty()) else {
            return Ok((self.default_engine.clone(), self.default_root.clone()));
        };

        // 1. Registry lookup by name (basename) — lets agents pass "lonaspark" not a full path
        let effective_root: PathBuf = if let Some(entry) = self.registry_lookup(arg) {
            info!(
                "[hermes] resolve: '{}' matched registry entry project_id={}",
                arg, entry.project_id
            );
            entry.canonical.clone()
        } else {
            PathBuf::from(arg)
        };

        // 2. Canonicalize the path (best-effort; keep original if path doesn't exist yet)
        let canonical = effective_root
            .canonicalize()
            .unwrap_or_else(|_| effective_root.clone());

        // 3. Default engine check
        let default_canonical = self
            .default_root
            .canonicalize()
            .unwrap_or_else(|_| self.default_root.clone());

        if canonical == default_canonical {
            info!(
                "[hermes] resolve: '{}' -> default project_id={}",
                arg,
                self.default_engine.project_id()
            );
            return Ok((self.default_engine.clone(), self.default_root.clone()));
        }

        // 4. Extra cache lookup
        let mut cache = self.extra.lock_ctx("resolve_extra")?;
        if let Some(engine) = cache.get(&canonical) {
            info!(
                "[hermes] resolve: '{}' -> cached project_id={}",
                arg,
                engine.project_id()
            );
            return Ok((engine.clone(), canonical));
        }

        // 5. Open a new engine, auto-index the directory
        let project_id = canonical
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();
        let engine = HermesEngine::new(&canonical.join(".hermes.db"), &project_id)?;
        info!(
            "[hermes] resolve: '{}' -> new project_id={} at {}, auto-indexing...",
            arg,
            project_id,
            canonical.display()
        );
        let graph = KnowledgeGraph::new(engine.db().clone(), &project_id);
        match IngestionPipeline::new(&graph).ingest_directory(&canonical) {
            Ok(r) => info!(
                "[hermes] auto-index {}: {} indexed, {} skipped, {} errors",
                project_id, r.indexed, r.skipped, r.errors
            ),
            Err(e) => warn!("[hermes] auto-index {} failed: {e}", project_id),
        }
        cache.insert(canonical.clone(), engine.clone());
        Ok((engine, canonical))
    }

    fn registry_lookup(&self, arg: &str) -> Option<&RegistryEntry> {
        // Exact project_id (basename) match
        if let Some(entry) = self.registry.iter().find(|e| e.project_id == arg) {
            return Some(entry);
        }
        // Canonical path match (arg is an absolute path already in the registry)
        let candidate = PathBuf::from(arg)
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(arg));
        self.registry.iter().find(|e| e.canonical == candidate)
    }

    pub(crate) fn list_projects(&self) -> Vec<Value> {
        let mut projects = vec![json!({
            "project_id": self.default_engine.project_id(),
            "project_root": self.default_root.display().to_string(),
            "source": "default"
        })];

        for entry in &self.registry {
            if entry.canonical
                != self
                    .default_root
                    .canonicalize()
                    .unwrap_or_else(|_| self.default_root.clone())
            {
                projects.push(json!({
                    "project_id": entry.project_id,
                    "project_root": entry.canonical.display().to_string(),
                    "source": "HERMES_PROJECTS"
                }));
            }
        }

        if let Ok(extra) = self.extra.lock() {
            let default_canonical = self
                .default_root
                .canonicalize()
                .unwrap_or_else(|_| self.default_root.clone());
            for (path, engine) in extra.iter() {
                let is_registry = self.registry.iter().any(|e| &e.canonical == path);
                if path != &default_canonical && !is_registry {
                    projects.push(json!({
                        "project_id": engine.project_id(),
                        "project_root": path.display().to_string(),
                        "source": "auto-discovered"
                    }));
                }
            }
        }

        projects
    }
}

// Parse HERMES_PROJECTS env var: semicolon-separated absolute paths.
// Example: HERMES_PROJECTS=D:\source\lonaspark;D:\source\hermes
pub(crate) fn parse_project_registry() -> Vec<RegistryEntry> {
    let Ok(raw) = std::env::var("HERMES_PROJECTS") else {
        return Vec::new();
    };
    raw.split(';')
        .filter(|s| !s.trim().is_empty())
        .filter_map(|s| {
            let path = PathBuf::from(s.trim());
            let canonical = path.canonicalize().unwrap_or_else(|_| path);
            let project_id = canonical
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
            Some(RegistryEntry { canonical, project_id })
        })
        .collect()
}

pub(crate) fn spawn_auto_reindex(engine: HermesEngine, project_root: PathBuf) {
    let interval_secs = std::env::var("HERMES_AUTO_INDEX_INTERVAL_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(300);

    if interval_secs == 0 {
        info!("[hermes] auto-reindex disabled (HERMES_AUTO_INDEX_INTERVAL_SECS=0)");
        return;
    }

    std::thread::spawn(move || {
        info!(
            "[hermes] auto-reindex thread started (interval={}s)",
            interval_secs
        );
        loop {
            std::thread::sleep(std::time::Duration::from_secs(interval_secs));
            let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
            let pipeline = IngestionPipeline::new(&graph);
            match pipeline.ingest_directory(&project_root) {
                Ok(report) => info!(
                    "[hermes] auto-reindex complete: {} indexed, {} skipped, {} errors",
                    report.indexed, report.skipped, report.errors
                ),
                Err(e) => warn!("[hermes] auto-reindex failed: {}", e),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HermesEngine;

    #[test]
    fn parse_registry_single_path() {
        std::env::set_var("HERMES_PROJECTS", "/tmp/test-project;/tmp/proj-b");
        let entries = parse_project_registry();
        assert!(entries.len() >= 1);
        assert_eq!(entries[0].project_id, "test-project");
    }

    #[test]
    fn list_projects_always_includes_default() {
        let engine = HermesEngine::in_memory("test-ec").unwrap();
        let root = std::path::PathBuf::from("/tmp/test-ec");
        let cache = EngineCache::new(engine, root, vec![]);
        let projects = cache.list_projects();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0]["source"], "default");
        assert_eq!(projects[0]["project_id"], "test-ec");
    }

    #[test]
    fn resolve_default_with_empty_arg() {
        let engine = HermesEngine::in_memory("test-ec-resolve").unwrap();
        let root = std::path::PathBuf::from("/tmp/test-ec-resolve");
        let cache = EngineCache::new(engine.clone(), root.clone(), vec![]);
        let (resolved_engine, resolved_root) = cache.resolve(None).unwrap();
        assert_eq!(resolved_engine.project_id(), engine.project_id());
        assert_eq!(resolved_root, root);
    }

    #[test]
    fn spawn_auto_reindex_disabled_when_zero() {
        std::env::set_var("HERMES_AUTO_INDEX_INTERVAL_SECS", "0");
        let engine = HermesEngine::in_memory("test-ec-reindex").unwrap();
        let root = std::path::PathBuf::from("/tmp/test-ec-reindex");
        // Should not spawn a thread; just return immediately
        spawn_auto_reindex(engine, root);
    }

    #[test]
    fn registry_lookup_by_name() {
        let engine = HermesEngine::in_memory("test-ec-reg").unwrap();
        let root = std::path::PathBuf::from("/tmp/default");
        let registry = vec![RegistryEntry {
            canonical: std::path::PathBuf::from("/tmp/other"),
            project_id: "other-proj".to_string(),
        }];
        let cache = EngineCache::new(engine, root, registry);
        assert!(cache.registry_lookup("other-proj").is_some());
        assert!(cache.registry_lookup("nonexistent").is_none());
    }

    #[test]
    fn resolve_with_temp_dir_creates_engine() {
        let engine = HermesEngine::in_memory("test-ec-resolve-tmp").unwrap();
        let root = std::path::PathBuf::from("/tmp/default");
        let cache = EngineCache::new(engine, root, vec![]);

        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("lib.rs"), "pub fn hello() {}").unwrap();

        let result = cache.resolve(Some(dir.path().to_str().unwrap()));
        assert!(result.is_ok());
    }

    #[test]
    fn resolve_with_name_from_registry() {
        let engine = HermesEngine::in_memory("test-ec-resolve-reg").unwrap();
        let root = std::path::PathBuf::from("/tmp/default");
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("lib.rs"), "pub fn hello() {}").unwrap();
        let can = std::fs::canonicalize(dir.path()).unwrap_or(dir.path().to_path_buf());
        let proj_id = can.file_name().unwrap().to_str().unwrap().to_string();
        let registry = vec![RegistryEntry {
            canonical: can.clone(),
            project_id: proj_id.clone(),
        }];
        let cache = EngineCache::new(engine, root, registry);
        let result = cache.resolve(Some(&proj_id));
        assert!(result.is_ok());
    }
}
