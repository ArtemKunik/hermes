// ChartApp/hermes-engine/src/ingestion/mod.rs
pub mod chunker;
pub mod crawler;
pub mod hash_tracker;

use crate::graph::{EdgeType, KnowledgeGraph, NodeType};
use anyhow::Result;
use rayon::prelude::*;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tracing::info;

pub struct IngestionPipeline<'a> {
    graph: &'a KnowledgeGraph,
    hash_tracker: hash_tracker::HashTracker<'a>,
}

impl<'a> IngestionPipeline<'a> {
    pub fn new(graph: &'a KnowledgeGraph) -> Self {
        Self {
            graph,
            hash_tracker: hash_tracker::HashTracker::new(graph.db(), graph.project_id()),
        }
    }

    pub fn ingest_directory(&self, dir_path: &Path) -> Result<IngestionReport> {
        let files = crawler::crawl_directory(dir_path)?;

        // Collect all crawled paths for stale-node cleanup later (Task 3.4)
        let crawled_paths: HashSet<String> = files
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();

        let mut report = IngestionReport {
            total_files: files.len(),
            ..Default::default()
        };

        // Serial pass: identify which files need re-ingestion
        let mut to_ingest: Vec<&PathBuf> = Vec::new();
        for file_path in &files {
            let path_str = file_path.to_string_lossy().to_string();
            if self.hash_tracker.is_unchanged(&path_str)? {
                report.skipped += 1;
            } else {
                to_ingest.push(file_path);
            }
        }

        // Task 3.2: Parallel ingestion — file reads and chunk processing run in parallel.
        // DB writes are serialized through the existing Arc<Mutex<Connection>>.
        let ingest_results: Vec<(String, Result<usize>)> = to_ingest
            .par_iter()
            .map(|file_path| {
                let path_str = file_path.to_string_lossy().to_string();
                let result = self.ingest_file(file_path);
                (path_str, result)
            })
            .collect();

        // Serial pass: aggregate results and update file-level hashes
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

        // Task 3.4: Stale node cleanup — remove DB entries for deleted/moved files
        self.cleanup_stale_nodes(&crawled_paths)?;

        Ok(report)
    }

    /// Task 3.4: Delete nodes for files no longer present on the filesystem.
    fn cleanup_stale_nodes(&self, crawled_paths: &HashSet<String>) -> Result<()> {
        let db_paths = self.graph.get_all_file_paths()?;
        for stale_path in db_paths.difference(crawled_paths) {
            self.graph.delete_nodes_for_file(stale_path)?;
            info!(path = %stale_path, "Removed stale nodes for deleted file");
        }
        Ok(())
    }

    pub fn ingest_file(&self, file_path: &Path) -> Result<usize> {
        let content = std::fs::read_to_string(file_path)?;
        let path_str = file_path.to_string_lossy().to_string();
        let chunks = chunker::chunk_file(file_path, &content);

        let file_hash = hash_tracker::compute_hash(&content);
        let file_node = self
            .graph
            .create_node_builder()
            .name(&path_str)
            .node_type(NodeType::File)
            .file_path(&path_str)
            .lines(1, content.lines().count() as i64)
            .content_hash(&file_hash)
            .build();

        self.graph.add_node(&file_node)?;
        self.graph.index_fts(&file_node, &content)?;

        let mut created = 1;

        for chunk in &chunks {
            // Task 2.2: Per-chunk hash dedup — skip re-inserting unchanged chunks
            let chunk_key = format!("{}::{}", path_str, chunk.name);
            let chunk_hash = hash_tracker::compute_hash(&chunk.content);

            if self.hash_tracker.is_chunk_unchanged(&chunk_key, &chunk_hash)? {
                continue;
            }

            let chunk_node = self
                .graph
                .create_node_builder()
                .name(&chunk.name)
                .node_type(chunk.node_type.clone())
                .file_path(&path_str)
                .lines(chunk.start_line as i64, chunk.end_line as i64)
                .summary(&chunk.summary)
                .build();

            self.graph.add_node(&chunk_node)?;
            self.graph.index_fts(&chunk_node, &chunk.content)?;

            let edge = self
                .graph
                .create_edge_builder()
                .source(&file_node.id)
                .target(&chunk_node.id)
                .edge_type(EdgeType::Contains)
                .build();

            self.graph.add_edge(&edge)?;
            self.hash_tracker.update_chunk_hash(&chunk_key, &chunk_hash)?;
            created += 1;
        }

        Ok(created)
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
    use crate::HermesEngine;
    use crate::graph::KnowledgeGraph;
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

        // First index
        pipeline.ingest_directory(dir.path()).unwrap();
        let paths_after_first = graph.get_all_file_paths().unwrap();
        assert!(!paths_after_first.is_empty());

        // Delete the file and re-index
        std::fs::remove_file(&file).unwrap();
        pipeline.ingest_directory(dir.path()).unwrap();
        let paths_after_second = graph.get_all_file_paths().unwrap();
        assert!(paths_after_second.is_empty());
    }
}

