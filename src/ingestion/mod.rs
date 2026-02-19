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

        let crawled_paths: HashSet<String> = files
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();

        let mut report = IngestionReport {
            total_files: files.len(),
            ..Default::default()
        };

        let mut to_ingest: Vec<&PathBuf> = Vec::new();
        for file_path in &files {
            let path_str = file_path.to_string_lossy().to_string();
            if self.hash_tracker.is_unchanged(&path_str)? {
                report.skipped += 1;
            } else {
                to_ingest.push(file_path);
            }
        }

        let ingest_results: Vec<(String, Result<usize>)> = to_ingest
            .par_iter()
            .map(|file_path| {
                let path_str = file_path.to_string_lossy().to_string();
                let result = self.ingest_file(file_path);
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

        self.cleanup_stale_nodes(&crawled_paths)?;

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

    pub fn ingest_file(&self, file_path: &Path) -> Result<usize> {
        // Read as raw bytes and convert to UTF-8 lossily so that files encoded
        // in Latin-1, Windows-1252, GBK, etc. are still indexed rather than
        // rejected with an "invalid UTF-8" error.
        let bytes = std::fs::read(file_path)?;
        let content = String::from_utf8_lossy(&bytes).into_owned();
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

        pipeline.ingest_directory(dir.path()).unwrap();
        let paths_after_first = graph.get_all_file_paths().unwrap();
        assert!(!paths_after_first.is_empty());

        std::fs::remove_file(&file).unwrap();
        pipeline.ingest_directory(dir.path()).unwrap();
        let paths_after_second = graph.get_all_file_paths().unwrap();
        assert!(paths_after_second.is_empty());
    }
}

