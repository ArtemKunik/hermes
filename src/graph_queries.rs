// ChartApp/hermes-engine/src/graph_queries.rs
use crate::graph::{KnowledgeGraph, Node, NodeType};
use anyhow::Result;
use rusqlite::params;
use std::collections::HashSet;

impl KnowledgeGraph {
    /// Task 1.1: SQL-backed literal search using LOWER(name) index.
    /// Tries prefix match first (index-friendly), falls back to contains.
    /// Never calls get_all_nodes().
    pub fn literal_search_by_name(&self, query: &str) -> Result<Vec<Node>> {
        self.with_conn(|conn| {
            let query_lower = query.to_lowercase();

            let prefix_pattern = format!("{}%", query_lower);
            let mut stmt = conn.prepare(
                "SELECT id, project_id, name, node_type, file_path, start_line, end_line, summary, content_tokens, object_type
                 FROM nodes WHERE project_id = ?1 AND LOWER(name) LIKE ?2",
            )?;
            let prefix_results: Vec<Node> = stmt
                .query_map(params![self.project_id(), prefix_pattern], node_from_row)?
                .collect::<std::result::Result<Vec<_>, _>>()?;

            if !prefix_results.is_empty() {
                return Ok(prefix_results);
            }

            let contains_pattern = format!("%{}%", query_lower);
            let mut stmt2 = conn.prepare(
                "SELECT id, project_id, name, node_type, file_path, start_line, end_line, summary, content_tokens, object_type
                 FROM nodes WHERE project_id = ?1 AND LOWER(name) LIKE ?2",
            )?;
            let results: Vec<Node> = stmt2
                .query_map(params![self.project_id(), contains_pattern], node_from_row)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(results)
        })
    }

    /// Task 3.4: Returns all distinct file paths stored for this project (file-type nodes).
    pub fn get_all_file_paths(&self) -> Result<HashSet<String>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT DISTINCT file_path FROM nodes
                 WHERE project_id = ?1 AND file_path IS NOT NULL",
            )?;
            let paths = stmt
                .query_map(params![self.project_id()], |row| row.get::<_, String>(0))?
                .collect::<std::result::Result<HashSet<_>, _>>()?;
            Ok(paths)
        })
    }

    /// Task 3.4: Delete all nodes, FTS entries, and edges for a given file path.
    pub fn delete_nodes_for_file(&self, file_path: &str) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "DELETE FROM fts_content WHERE node_id IN
                 (SELECT id FROM nodes WHERE file_path = ?1 AND project_id = ?2)",
                params![file_path, self.project_id()],
            )?;
            conn.execute(
                "DELETE FROM edges WHERE
                 source_id IN (SELECT id FROM nodes WHERE file_path = ?1 AND project_id = ?2)
                 OR target_id IN (SELECT id FROM nodes WHERE file_path = ?1 AND project_id = ?2)",
                params![file_path, self.project_id()],
            )?;
            conn.execute(
                "DELETE FROM nodes WHERE file_path = ?1 AND project_id = ?2",
                params![file_path, self.project_id()],
            )?;
            let hash_prefix = format!("{file_path}::%");
            conn.execute(
                "DELETE FROM file_hashes WHERE
                 (file_path = ?1 OR file_path LIKE ?2) AND project_id = ?3",
                params![file_path, hash_prefix, self.project_id()],
            )?;
            Ok(())
        })
    }

    pub fn get_all_nodes(&self) -> Result<Vec<Node>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, project_id, name, node_type, file_path, start_line, end_line, summary, content_tokens, object_type
                 FROM nodes WHERE project_id = ?1",
            )?;
            let rows = stmt
                .query_map(params![self.project_id()], node_from_row)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    pub fn fts_search(&self, query: &str, limit: usize) -> Result<Vec<(Node, f64)>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT n.id, n.project_id, n.name, n.node_type, n.file_path, n.start_line, n.end_line, n.summary, n.content_tokens, n.object_type,
                        bm25(fts_content) as rank
                 FROM fts_content f
                 JOIN nodes n ON n.id = f.node_id
                 WHERE fts_content MATCH ?1 AND f.project_id = ?2
                 ORDER BY rank
                 LIMIT ?3",
            )?;
            let rows = stmt
                .query_map(params![query, self.project_id(), limit as i64], |row| {
                    Ok((node_from_row(row)?, row.get::<_, f64>(10)?))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }
}

pub(crate) fn node_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Node> {
    Ok(Node {
        id: row.get(0)?,
        project_id: row.get(1)?,
        name: row.get(2)?,
        node_type: NodeType::parse_str(&row.get::<_, String>(3)?),
        file_path: row.get(4)?,
        start_line: row.get(5)?,
        end_line: row.get(6)?,
        summary: row.get(7)?,
        content_hash: None,
        content_tokens: row.get::<_, Option<i64>>(8)?.map(|v| v as u64),
        object_type: row.get(9)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{graph::KnowledgeGraph, graph_builders::NodeBuilder, HermesEngine};
    use rusqlite::params;

    #[test]
    fn delete_nodes_for_file_removes_file_hashes_and_chunk_hashes() {
        let engine = HermesEngine::in_memory("test-project").unwrap();
        let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
        let file_node = NodeBuilder::new("test-project")
            .name("test.rs")
            .node_type(NodeType::File)
            .file_path("test.rs")
            .build();
        graph.add_node(&file_node).unwrap();

        let conn = engine.db().lock().unwrap();
        conn.execute(
            "INSERT INTO file_hashes (file_path, project_id, indexed_at)
             VALUES (?1, ?2, datetime('now'))",
            params!["test.rs", engine.project_id()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO file_hashes (file_path, project_id, indexed_at)
             VALUES (?1, ?2, datetime('now'))",
            params!["test.rs::some_symbol", engine.project_id()],
        )
        .unwrap();
        drop(conn);

        graph.delete_nodes_for_file("test.rs").unwrap();

        let conn = engine.db().lock().unwrap();
        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM file_hashes WHERE project_id = ?1",
                params![engine.project_id()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 0);
    }

    #[test]
    fn get_all_file_paths_returns_paths_from_non_file_nodes() {
        let engine = HermesEngine::in_memory("test-nonfile-paths").unwrap();
        let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());

        let func_node = NodeBuilder::new(engine.project_id())
            .name("test_function")
            .node_type(NodeType::Function)
            .file_path("/project/src/lib.rs")
            .build();
        graph.add_node(&func_node).unwrap();

        let struct_node = NodeBuilder::new(engine.project_id())
            .name("TestStruct")
            .node_type(NodeType::Struct)
            .file_path("/project/src/lib.rs")
            .build();
        graph.add_node(&struct_node).unwrap();

        let paths = graph.get_all_file_paths().unwrap();
        assert!(
            paths.contains("/project/src/lib.rs"),
            "get_all_file_paths should return paths from Function/Struct nodes, got: {paths:?}"
        );
    }

    #[test]
    fn get_all_file_paths_deduplicates_shared_paths() {
        let engine = HermesEngine::in_memory("test-dedup-paths").unwrap();
        let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());

        let file_node = NodeBuilder::new(engine.project_id())
            .name("lib.rs")
            .node_type(NodeType::File)
            .file_path("/project/src/lib.rs")
            .build();
        graph.add_node(&file_node).unwrap();

        let func_node = NodeBuilder::new(engine.project_id())
            .name("helper")
            .node_type(NodeType::Function)
            .file_path("/project/src/lib.rs")
            .build();
        graph.add_node(&func_node).unwrap();

        let paths = graph.get_all_file_paths().unwrap();
        assert_eq!(paths.len(), 1, "same path from different node types should be deduplicated");
        assert!(paths.contains("/project/src/lib.rs"));
    }
}
