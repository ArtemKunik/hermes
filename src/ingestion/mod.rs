pub mod chunker;
pub mod crawler;
pub mod env_scanner;
pub mod hash_tracker;

use crate::graph::{ChunkWriteRecord, EdgeType, KnowledgeGraph, NodeType};
use anyhow::Result;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tracing::info;

pub struct IngestionPipeline<'a> {
    graph: &'a KnowledgeGraph,
    hash_tracker: hash_tracker::HashTracker<'a>,
    env_scanner: env_scanner::EnvScanner,
}

struct LoadedFile {
    path: PathBuf,
    path_str: String,
    content: String,
    file_hash: String,
}

impl<'a> IngestionPipeline<'a> {
    pub fn new(graph: &'a KnowledgeGraph) -> Self {
        Self {
            graph,
            hash_tracker: hash_tracker::HashTracker::new(graph.db(), graph.project_id()),
            env_scanner: env_scanner::EnvScanner::new()
                .expect("env_scanner regex compilation must not fail"),
        }
    }

    pub fn ingest_directory(&self, dir_path: &Path) -> Result<IngestionReport> {
        self.ingest_directory_impl(dir_path, false)
    }

    pub fn ingest_directory_force(&self, dir_path: &Path) -> Result<IngestionReport> {
        self.ingest_directory_impl(dir_path, true)
    }

    fn ingest_directory_impl(&self, dir_path: &Path, force: bool) -> Result<IngestionReport> {
        if force {
            self.hash_tracker.clear_all_hashes()?;
        }
        let files = crawler::crawl_directory(dir_path)?;
        let loaded_files = Self::load_files(&files)?;
        let stored_hashes = self.hash_tracker.load_all_hashes()?;

        let crawled_paths: HashSet<String> = loaded_files
            .iter()
            .map(|file| file.path_str.clone())
            .collect();

        // TRACK-040: Scan all files for env var usage/definitions → config_registry.
        self.scan_and_populate_env_vars(&loaded_files)?;

        let mut report = IngestionReport {
            total_files: loaded_files.len(),
            ..Default::default()
        };

        let mut to_ingest: Vec<&LoadedFile> = Vec::new();
        for file in &loaded_files {
            if stored_hashes.get(&file.path_str) == Some(&file.file_hash) {
                report.skipped += 1;
            } else {
                to_ingest.push(file);
            }
        }

        let ingest_results: Vec<(String, Result<usize>)> = to_ingest
            .par_iter()
            .map(|file| {
                let result = self.ingest_loaded_file(file, &stored_hashes);
                (file.path_str.clone(), result)
            })
            .collect();

        for (path_str, result) in ingest_results {
            match result {
                Ok(count) => {
                    report.indexed += 1;
                    report.nodes_created += count;
                }
                Err(e) => {
                    info!(path = %path_str, error = %e, "Failed to ingest file");
                    report.errors += 1;
                }
            }
        }

        self.cleanup_stale_nodes(&crawled_paths)?;

        Ok(report)
    }

    /// TRACK-040: Scan every crawled file for env var references and populate config_registry.
    fn scan_and_populate_env_vars(&self, files: &[LoadedFile]) -> Result<()> {
        let discovered: Vec<env_scanner::DiscoveredEnvVar> = files
            .par_iter()
            .flat_map(|file| self.env_scanner.scan_file(&file.path, &file.content))
            .collect();

        let conn = self.graph.db().lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        self.env_scanner
            .populate_registry(&conn, self.graph.project_id(), &discovered)?;
        info!(
            count = discovered.len(),
            "Populated config_registry with discovered env vars"
        );
        Ok(())
    }

    fn cleanup_stale_nodes(&self, crawled_paths: &HashSet<String>) -> Result<()> {
        let db_paths = self.graph.get_all_file_paths()?;
        for stale_path in db_paths.difference(crawled_paths) {
            self.graph.delete_nodes_for_file(stale_path)?;
            info!(path = %stale_path, "Removed stale nodes for deleted file");
        }
        Ok(())
    }

    pub fn ingest_file(&self, file_path: &Path) -> Result<usize> {
        let loaded_file = Self::load_file(file_path.to_path_buf())?;
        let stored_hashes = self.hash_tracker.load_all_hashes()?;
        self.ingest_loaded_file(&loaded_file, &stored_hashes)
    }

    fn ingest_loaded_file(
        &self,
        file: &LoadedFile,
        stored_hashes: &HashMap<String, String>,
    ) -> Result<usize> {
        let chunks = chunker::chunk_file(&file.path, &file.content);
        let file_node = self
            .graph
            .create_node_builder()
            .name(&file.path_str)
            .node_type(NodeType::File)
            .file_path(&file.path_str)
            .lines(1, file.content.lines().count() as i64)
            .content_hash(&file.file_hash)
            .build();

        // Phase 1: determine which chunks changed using the hash snapshot loaded
        // once at the start of the indexing pass.
        let mut chunk_records: Vec<ChunkWriteRecord> = Vec::new();
        for chunk in &chunks {
            let chunk_key = format!("{}::{}", file.path_str, chunk.name);
            let chunk_hash = hash_tracker::compute_hash(&chunk.content);
            if stored_hashes.get(&chunk_key) == Some(&chunk_hash) {
                continue;
            }
            let chunk_node = self
                .graph
                .create_node_builder()
                .name(&chunk.name)
                .node_type(chunk.node_type.clone())
                .file_path(&file.path_str)
                .lines(chunk.start_line as i64, chunk.end_line as i64)
                .summary(&chunk.summary)
                .build();
            let edge = self
                .graph
                .create_edge_builder()
                .source(&file_node.id)
                .target(&chunk_node.id)
                .edge_type(EdgeType::Contains)
                .build();
            chunk_records.push(ChunkWriteRecord {
                node: chunk_node,
                content: chunk.content.clone(),
                edge,
                hash_key: chunk_key,
                hash_value: chunk_hash,
            });
        }

        let created = 1 + chunk_records.len();

        // Phase 2: write the file node, all chunk nodes/edges, and chunk hashes
        // in a single transaction — one lock acquisition and one BEGIN/COMMIT.
        self.graph
            .ingest_file_batch(&file_node, &file.content, &chunk_records)?;

        Ok(created)
    }

    fn load_files(paths: &[PathBuf]) -> Result<Vec<LoadedFile>> {
        let results: Vec<Result<LoadedFile>> = paths
            .par_iter()
            .map(|path| Self::load_file(path.clone()))
            .collect();
        results.into_iter().collect()
    }

    fn load_file(path: PathBuf) -> Result<LoadedFile> {
        // Read as raw bytes and convert to UTF-8 lossily so that files encoded
        // in Latin-1, Windows-1252, GBK, etc. are still indexed rather than
        // rejected with an "invalid UTF-8" error.
        let bytes = std::fs::read(&path)?;
        let content = String::from_utf8_lossy(&bytes).into_owned();
        let path_str = path.to_string_lossy().to_string();
        let file_hash = hash_tracker::compute_hash(&content);
        Ok(LoadedFile {
            path,
            path_str,
            content,
            file_hash,
        })
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
mod tests {
    use super::*;
    use crate::graph::KnowledgeGraph;
    use crate::HermesEngine;
    use tempfile::TempDir;

    fn make_graph_for(engine: &HermesEngine) -> KnowledgeGraph {
        KnowledgeGraph::new(engine.db().clone(), engine.project_id())
    }

    #[test]
    fn test_ingest_empty_dir_returns_zero_report() {
        let dir = TempDir::new().unwrap();
        let engine = HermesEngine::in_memory("test-ingest").unwrap();
        let graph = make_graph_for(&engine);
        let pipeline = IngestionPipeline::new(&graph);
        let report = pipeline.ingest_directory(dir.path()).unwrap();
        assert_eq!(report.total_files, 0);
        assert_eq!(report.nodes_created, 0);
    }

    #[test]
    fn test_unchanged_file_is_skipped_on_reindex() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("test.rs");
        std::fs::write(&file, "fn main() {}").unwrap();

        let engine = HermesEngine::in_memory("test-skip").unwrap();
        let graph = make_graph_for(&engine);
        let pipeline = IngestionPipeline::new(&graph);

        let report1 = pipeline.ingest_directory(dir.path()).unwrap();
        assert_eq!(report1.indexed, 1);
        assert_eq!(report1.skipped, 0);

        let report2 = pipeline.ingest_directory(dir.path()).unwrap();
        assert_eq!(report2.indexed, 0);
        assert_eq!(report2.skipped, 1);
    }

    #[test]
    fn test_stale_file_removed_after_deletion() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("will_be_deleted.rs");
        std::fs::write(&file, "fn foo() {}").unwrap();

        let engine = HermesEngine::in_memory("test-stale").unwrap();
        let graph = make_graph_for(&engine);
        let pipeline = IngestionPipeline::new(&graph);

        pipeline.ingest_directory(dir.path()).unwrap();
        let paths_after_first = graph.get_all_file_paths().unwrap();
        assert!(!paths_after_first.is_empty());

        std::fs::remove_file(&file).unwrap();
        pipeline.ingest_directory(dir.path()).unwrap();
        let paths_after_second = graph.get_all_file_paths().unwrap();
        assert!(paths_after_second.is_empty());
    }
}
