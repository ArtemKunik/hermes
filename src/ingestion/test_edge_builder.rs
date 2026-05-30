// tools/hermes-engine/src/ingestion/test_edge_builder.rs
// TRACK-045: Build Tests edges from test files to their implementation targets.

use anyhow::Result;
use rusqlite::params;
use std::path::Path;

use crate::graph::{EdgeType, KnowledgeGraph};
use crate::graph_builders::EdgeBuilder;

/// Scan the graph for test file nodes, infer their implementation targets,
/// and insert `Tests` edges. Returns the number of edges created.
pub fn build_test_edges(graph: &KnowledgeGraph, _project_root: &Path) -> Result<usize> {
    let test_files = find_test_files(graph)?;
    let mut created = 0;

    for (test_node_id, test_file_path) in &test_files {
        let impl_targets = resolve_impl_targets(graph, test_file_path)?;
        for target_id in impl_targets {
            let edge = EdgeBuilder::new(graph.project_id())
                .source(test_node_id)
                .target(&target_id)
                .edge_type(EdgeType::Tests)
                .weight(1.0)
                .build();
            match graph.add_edge(&edge) {
                Ok(()) => created += 1,
                Err(e) => eprintln!("[test-edge-builder] failed to add edge: {e}"),
            }
        }
    }
    Ok(created)
}

/// Return (node_id, file_path) for all file nodes that look like test files.
fn find_test_files(graph: &KnowledgeGraph) -> Result<Vec<(String, String)>> {
    graph.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT id, file_path FROM nodes
             WHERE project_id = ?1
               AND node_type = 'file'
               AND file_path IS NOT NULL
               AND (file_path LIKE '%_test.rs'
                    OR file_path LIKE '%.test.tsx'
                    OR file_path LIKE '%.test.ts'
                    OR file_path LIKE '%.spec.ts'
                    OR file_path LIKE '%.spec.tsx'
                    OR file_path LIKE '%/tests/%'
                    OR file_path LIKE '%\\tests\\%')",
        )?;
        let results: Vec<(String, String)> = stmt
            .query_map(params![graph.project_id()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(results)
    })
}

/// Given a test file path, find likely implementation node IDs.
///
/// Strategy (in order of preference):
/// 1. Name-based: strip _test / .test / .spec suffix → match file node by name
/// 2. Import-based: look for `Imports` edges FROM this test file node to impl nodes
fn resolve_impl_targets(graph: &KnowledgeGraph, test_path: &str) -> Result<Vec<String>> {
    let mut targets = Vec::new();

    // Strategy 1: name-based matching
    if let Some(impl_id) = impl_by_name(graph, test_path)? {
        targets.push(impl_id);
    }

    // Strategy 2: import-based
    let import_targets = impl_by_imports(graph, test_path)?;
    for id in import_targets {
        if !targets.contains(&id) {
            targets.push(id);
        }
    }

    Ok(targets)
}

fn strip_test_suffix(file_path: &str) -> Option<String> {
    let norm = file_path.replace('\\', "/");
    // Extract just the filename
    let filename = norm.split('/').last()?;

    let base = filename
        .strip_suffix("_test.rs")
        .or_else(|| filename.strip_suffix(".test.tsx"))
        .or_else(|| filename.strip_suffix(".test.ts"))
        .or_else(|| filename.strip_suffix(".spec.ts"))
        .or_else(|| filename.strip_suffix(".spec.tsx"))?;

    Some(base.to_string())
}

fn impl_by_name(graph: &KnowledgeGraph, test_path: &str) -> Result<Option<String>> {
    let Some(base_name) = strip_test_suffix(test_path) else {
        return Ok(None);
    };

    graph.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT id FROM nodes
             WHERE project_id = ?1
               AND node_type = 'file'
               AND file_path IS NOT NULL
               AND file_path NOT LIKE '%_test.rs'
               AND file_path NOT LIKE '%.test.%'
               AND file_path NOT LIKE '%.spec.%'
               AND (name = ?2 OR file_path LIKE '%/' || ?2 || '.rs'
                    OR file_path LIKE '%/' || ?2 || '.ts'
                    OR file_path LIKE '%/' || ?2 || '.tsx')
             LIMIT 1",
        )?;
        let result: Option<String> = stmt
            .query_row(params![graph.project_id(), base_name], |row| row.get(0))
            .ok();
        Ok(result)
    })
}

fn impl_by_imports(graph: &KnowledgeGraph, test_path: &str) -> Result<Vec<String>> {
    graph.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT e.target_id FROM edges e
             JOIN nodes src ON src.id = e.source_id
             JOIN nodes tgt ON tgt.id = e.target_id
             WHERE src.project_id = ?1
               AND src.file_path = ?2
               AND e.edge_type = 'imports'
               AND tgt.node_type = 'file'
               AND tgt.file_path NOT LIKE '%_test.rs'
               AND tgt.file_path NOT LIKE '%.test.%'",
        )?;
        let results: Vec<String> = stmt
            .query_map(params![graph.project_id(), test_path], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(results)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_test_suffix_rs() {
        assert_eq!(
            strip_test_suffix("src/task_service_test.rs").as_deref(),
            Some("task_service")
        );
    }

    #[test]
    fn strip_test_suffix_tsx() {
        assert_eq!(
            strip_test_suffix("src/TaskList.test.tsx").as_deref(),
            Some("TaskList")
        );
    }

    #[test]
    fn strip_test_suffix_spec() {
        assert_eq!(
            strip_test_suffix("src/api.spec.ts").as_deref(),
            Some("api")
        );
    }

    #[test]
    fn build_edges_on_empty_graph() {
        let engine = crate::HermesEngine::in_memory("test-edge").unwrap();
        let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
        let dir = tempfile::tempdir().unwrap();
        let n = build_test_edges(&graph, dir.path()).unwrap();
        assert_eq!(n, 0);
    }
}
