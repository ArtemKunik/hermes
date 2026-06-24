// ChartApp/hermes-engine/src/ingestion/mod.rs
pub mod ast_chunker;
pub mod chunker;
pub mod crawler;
pub mod env_scanner;
pub mod file_ops;
pub mod hash_tracker;
pub mod llm_enricher;
pub mod skill_scanner;
pub mod test_edge_builder;
pub mod xref_extractor;

use crate::graph::{EdgeType, KnowledgeGraph, NodeType};
use crate::ingestion::env_scanner::EnvScanner;
use anyhow::Result;
use llm_enricher::LlmEnricher;
use rayon::prelude::*;
#[cfg(feature = "ast")]
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tracing::info;

pub struct IngestionPipeline<'a> {
    graph: &'a KnowledgeGraph,
    hash_tracker: hash_tracker::HashTracker<'a>,
    enricher: Option<Arc<LlmEnricher>>,
    env_scanner: env_scanner::EnvScanner,
}

impl<'a> IngestionPipeline<'a> {
    pub fn new(graph: &'a KnowledgeGraph) -> Self {
        Self {
            graph,
            hash_tracker: hash_tracker::HashTracker::new(graph.db(), graph.project_id()),
            enricher: None,
            env_scanner: env_scanner::EnvScanner::new().unwrap(),
        }
    }

    pub fn with_enricher(mut self, enricher: LlmEnricher) -> Self {
        self.enricher = Some(Arc::new(enricher));
        self
    }

    pub fn ingest_directory(&self, dir_path: &Path) -> Result<IngestionReport> {
        let files = crawler::crawl_directory(dir_path)?;

        let crawled_paths: HashSet<String> = files
            .iter()
            .map(|p| hash_tracker::normalize_logical_path(&p.to_string_lossy()))
            .collect();

        self.scan_and_populate_skills(dir_path)?;

        let mut report = IngestionReport {
            total_files: files.len(),
            ..Default::default()
        };

        // Load all stored hashes in one query rather than one query per file.
        // This replaces N Mutex acquisitions (one per file) with a single acquisition,
        // eliminating the contention that caused tool calls to block for >30s on Windows.
        let stored_hashes = self.hash_tracker.load_all_hashes()?;

        let mut to_ingest: Vec<&PathBuf> = Vec::new();
        for file_path in &files {
            let path_str = hash_tracker::normalize_logical_path(&file_path.to_string_lossy());
            let is_unchanged = if let Some(stored) = stored_hashes.get(&path_str) {
                match std::fs::read(file_path) {
                    Ok(raw) => {
                        let content = String::from_utf8_lossy(&raw).into_owned();
                        hash_tracker::compute_hash(&content) == *stored
                    }
                    Err(_) => false,
                }
            } else {
                false
            };
            if is_unchanged {
                report.skipped += 1;
            } else {
                to_ingest.push(file_path);
            }
        }

        {
            let conn = self.graph.db().lock().map_err(|e| anyhow::anyhow!("{e}"))?;
            conn.execute_batch("BEGIN IMMEDIATE")?;
        }

        // Purge every stale node (and its FTS entry) for files that are about
        // to be re-indexed.  Without this, each indexing run for a changed file
        // appends new nodes while old ones accumulate, bloating the DB unboundedly.
        for file_path in &to_ingest {
            let path_str = hash_tracker::normalize_logical_path(&file_path.to_string_lossy());
            let _ = self.graph.delete_nodes_for_file(&path_str);
        }

        let env_vars_acc: Arc<Mutex<Vec<env_scanner::DiscoveredEnvVar>>> =
            Arc::new(Mutex::new(Vec::new()));

        let env_acc = Arc::clone(&env_vars_acc);
        let ingest_results: Vec<(String, Result<usize>)> = to_ingest
            .par_iter()
            .map(|file_path| {
                let path_str = hash_tracker::normalize_logical_path(&file_path.to_string_lossy());
                let result = self.ingest_file(file_path, &env_acc);
                (path_str, result)
            })
            .collect();

        for (path_str, result) in ingest_results {
            match result {
                Ok(count) => {
                    report.indexed += 1;
                    report.nodes_created += count;
                    let p = PathBuf::from(&path_str);
                    self.hash_tracker.update_hash(&path_str, &p)?;
                }
                Err(e) => {
                    info!(path = %path_str, error = %e, "Failed to ingest file");
                    report.errors += 1;
                }
            }
        }

        {
            let discovered = env_vars_acc.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
            let conn = self.graph.db().lock().map_err(|e| anyhow::anyhow!("{e}"))?;
            self.env_scanner
                .populate_registry(&conn, self.graph.project_id(), &discovered)?;
            info!(
                count = discovered.len(),
                "Populated config_registry with discovered environment variables"
            );
        }

        self.cleanup_stale_nodes(&crawled_paths)?;

        #[cfg(feature = "ast")]
        {
            let name_to_id = self.build_name_to_id_map()?;
            for file_path in &to_ingest {
                if self.is_ast_supported_file(file_path) {
                    if let Ok(raw) = std::fs::read(file_path) {
                        let content = String::from_utf8_lossy(&raw);
                        let path_str =
                            hash_tracker::normalize_logical_path(&file_path.to_string_lossy());
                        let file_node_id = self
                            .graph
                            .create_node_builder()
                            .name(&path_str)
                            .node_type(NodeType::File)
                            .build()
                            .id;
                        let _ = self.extract_and_store_xrefs(
                            &content,
                            &path_str,
                            &file_node_id,
                            &name_to_id,
                        );
                    }
                }
            }
        }

        {
            let conn = self.graph.db().lock().map_err(|e| anyhow::anyhow!("{e}"))?;
            conn.execute_batch("COMMIT")?;
        }

        if report.indexed > 0 {
            let conn = self.graph.db().lock().map_err(|e| anyhow::anyhow!("{e}"))?;
            let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)");
        }

        Ok(report)
    }

    fn cleanup_stale_nodes(&self, crawled_paths: &HashSet<String>) -> Result<()> {
        let db_paths = self.graph.get_all_file_paths()?;
        for stale_path in db_paths.difference(crawled_paths) {
            self.graph.delete_nodes_for_file(stale_path)?;
            info!(path = %stale_path, "Removed stale nodes for deleted file");
        }
        Ok(())
    }

    fn scan_and_populate_skills(&self, project_root: &Path) -> Result<()> {
        let skills = skill_scanner::discover_skills(project_root);
        let conn = self.graph.db().lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        skill_scanner::populate_skills(&conn, self.graph.project_id(), &skills)?;
        info!(count = skills.len(), "Indexed skill assets");
        Ok(())
    }

    #[cfg(feature = "ast")]
    fn is_ast_supported_file(&self, file_path: &Path) -> bool {
        file_path.extension().and_then(|s| s.to_str()) == Some("rs")
    }

    #[cfg(feature = "ast")]
    fn build_name_to_id_map(&self) -> Result<HashMap<String, String>> {
        let known_nodes = self.graph.get_all_nodes()?;
        Ok(known_nodes.into_iter().map(|n| (n.name, n.id)).collect())
    }

    #[cfg(feature = "ast")]
    fn extract_and_store_xrefs(
        &self,
        content: &str,
        file_path: &str,
        file_node_id: &str,
        name_to_id: &HashMap<String, String>,
    ) -> Result<()> {
        let mut extractor = match xref_extractor::XrefExtractor::new() {
            Ok(e) => e,
            Err(_) => return Ok(()),
        };
        let xrefs = extractor.extract(content, file_path);
        if xrefs.is_empty() {
            return Ok(());
        }
        for xref in xrefs {
            let Some(target_id) = name_to_id.get(&xref.to_name) else {
                continue;
            };
            let source_id = name_to_id
                .get(&xref.from_name)
                .map(|id| id.as_str())
                .unwrap_or(file_node_id);
            let edge_type = match xref.kind {
                xref_extractor::XrefKind::Calls => EdgeType::Calls,
                xref_extractor::XrefKind::Imports => EdgeType::Imports,
            };
            let edge = self
                .graph
                .create_edge_builder()
                .source(source_id)
                .target(target_id)
                .edge_type(edge_type)
                .build();
            self.graph.add_edge(&edge)?;
        }
        Ok(())
    }

    pub fn ingest_file(
        &self,
        file_path: &Path,
        env_vars_acc: &Arc<Mutex<Vec<env_scanner::DiscoveredEnvVar>>>,
    ) -> Result<usize> {
        file_ops::ingest_file_inner(
            self.graph,
            &self.hash_tracker,
            self.enricher.as_ref(),
            &self.env_scanner,
            file_path,
            env_vars_acc,
        )
    }

    pub fn backfill_content_tokens(&self) -> Result<usize> {
        file_ops::backfill_content_tokens_inner(self.graph)
    }
}

#[derive(Debug, Default)]
pub struct IngestionReport {
    pub total_files: usize,
    pub indexed: usize,
    pub skipped: usize,
    pub errors: usize,
    pub nodes_created: usize,
}

impl std::fmt::Display for IngestionReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Ingestion: {} files ({} indexed, {} skipped, {} errors), {} nodes",
            self.total_files, self.indexed, self.skipped, self.errors, self.nodes_created
        )
    }
}

#[cfg(test)]
mod tests;
