use crate::graph::{KnowledgeGraph, Node, NodeType};
use anyhow::Result;
use rusqlite::params;
use std::collections::HashSet;

impl KnowledgeGraph {
    pub fn literal_search_by_name(&self, query: &str) -> Result<Vec<Node>> {
        let conn = self.db().lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let query_lower = query.to_lowercase();

        let prefix_pattern = format!("{}%", query_lower);
        let mut stmt = conn.prepare(
            "SELECT id, project_id, name, node_type, file_path, start_line, end_line, summary, content_hash
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
            "SELECT id, project_id, name, node_type, file_path, start_line, end_line, summary, content_hash
             FROM nodes WHERE project_id = ?1 AND LOWER(name) LIKE ?2",
        )?;
        let results: Vec<Node> = stmt2
            .query_map(params![self.project_id(), contains_pattern], node_from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(results)
    }

    pub fn get_all_file_paths(&self) -> Result<HashSet<String>> {
        let conn = self.db().lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut stmt = conn.prepare(
            "SELECT DISTINCT file_path FROM nodes
             WHERE project_id = ?1 AND node_type = 'file' AND file_path IS NOT NULL",
        )?;
        let paths = stmt
            .query_map(params![self.project_id()], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<HashSet<_>, _>>()?;
        Ok(paths)
    }

    pub fn delete_nodes_for_file(&self, file_path: &str) -> Result<()> {
        let conn = self.db().lock().map_err(|e| anyhow::anyhow!("{e}"))?;
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
        Ok(())
    }

    pub fn get_all_nodes(&self) -> Result<Vec<Node>> {
        let conn = self.db().lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut stmt = conn.prepare(
            "SELECT id, project_id, name, node_type, file_path, start_line, end_line, summary, content_hash
             FROM nodes WHERE project_id = ?1",
        )?;
        let rows = stmt
            .query_map(params![self.project_id()], node_from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn fts_search(&self, query: &str, limit: usize) -> Result<Vec<(Node, f64)>> {
        let conn = self.db().lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut stmt = conn.prepare(
            "SELECT n.id, n.project_id, n.name, n.node_type, n.file_path, n.start_line, n.end_line, n.summary, n.content_hash,
                    bm25(fts_content) as rank
             FROM fts_content f
             JOIN nodes n ON n.id = f.node_id
             WHERE fts_content MATCH ?1 AND f.project_id = ?2
             ORDER BY rank
             LIMIT ?3",
        )?;
        let rows = stmt
            .query_map(params![query, self.project_id(), limit as i64], |row| {
                Ok((node_from_row(row)?, row.get::<_, f64>(9)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        graph::{Edge, EdgeType, KnowledgeGraph, Node, NodeType},
        HermesEngine,
    };

    fn make_graph(engine: &HermesEngine) -> KnowledgeGraph {
        KnowledgeGraph::new(engine.db().clone(), engine.project_id())
    }

    fn insert_node(graph: &KnowledgeGraph, id: &str, name: &str, file_path: &str) -> Node {
        let node = Node {
            id: id.to_string(),
            project_id: graph.project_id().to_string(),
            name: name.to_string(),
            node_type: NodeType::Function,
            file_path: Some(file_path.to_string()),
            start_line: Some(1),
            end_line: Some(10),
            summary: None,
            content_hash: None,
        };
        graph.add_node(&node).unwrap();
        node
    }

    // ── literal_search_by_name ───────────────────────────────────────────────

    #[test]
    fn literal_search_prefix_match() {
        let engine = HermesEngine::in_memory("gq-literal").unwrap();
        let graph = make_graph(&engine);
        insert_node(&graph, "n1", "fetch_alerts", "src/api.rs");
        insert_node(&graph, "n2", "process_alerts", "src/api.rs");

        let results = graph.literal_search_by_name("fetch").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "fetch_alerts");
    }

    #[test]
    fn literal_search_contains_fallback() {
        let engine = HermesEngine::in_memory("gq-contains").unwrap();
        let graph = make_graph(&engine);
        insert_node(&graph, "n1", "fetch_alerts_handler", "src/api.rs");

        // "alerts" is not a prefix but is contained in the name
        let results = graph.literal_search_by_name("alerts").unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].name, "fetch_alerts_handler");
    }

    #[test]
    fn literal_search_is_case_insensitive() {
        let engine = HermesEngine::in_memory("gq-case").unwrap();
        let graph = make_graph(&engine);
        insert_node(&graph, "n1", "HandleRequest", "src/server.rs");

        let results = graph.literal_search_by_name("handlerequest").unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn literal_search_returns_empty_for_no_match() {
        let engine = HermesEngine::in_memory("gq-nomatch").unwrap();
        let graph = make_graph(&engine);
        insert_node(&graph, "n1", "my_func", "src/lib.rs");

        let results = graph.literal_search_by_name("nonexistent_xyz").unwrap();
        assert!(results.is_empty());
    }

    // ── get_all_nodes ────────────────────────────────────────────────────────────

    #[test]
    fn get_all_nodes_empty() {
        let engine = HermesEngine::in_memory("gq-allnodes-empty").unwrap();
        let graph = make_graph(&engine);
        assert!(graph.get_all_nodes().unwrap().is_empty());
    }

    #[test]
    fn get_all_nodes_returns_inserted_nodes() {
        let engine = HermesEngine::in_memory("gq-allnodes").unwrap();
        let graph = make_graph(&engine);
        insert_node(&graph, "n1", "alpha", "src/a.rs");
        insert_node(&graph, "n2", "beta", "src/b.rs");

        let all = graph.get_all_nodes().unwrap();
        assert_eq!(all.len(), 2);
    }

    // ── get_all_file_paths ──────────────────────────────────────────────────────

    #[test]
    fn get_all_file_paths_only_returns_file_type_nodes() {
        let engine = HermesEngine::in_memory("gq-filepaths").unwrap();
        let graph = make_graph(&engine);

        // Add a File-typed node
        let file_node = Node {
            id: "file-1".to_string(),
            project_id: graph.project_id().to_string(),
            name: "src/main.rs".to_string(),
            node_type: NodeType::File,
            file_path: Some("src/main.rs".to_string()),
            start_line: None,
            end_line: None,
            summary: None,
            content_hash: None,
        };
        graph.add_node(&file_node).unwrap();

        // Add a Function-typed node (must NOT appear in file paths)
        insert_node(&graph, "fn-1", "some_fn", "src/main.rs");

        let paths = graph.get_all_file_paths().unwrap();
        assert_eq!(paths.len(), 1);
        assert!(paths.contains("src/main.rs"));
    }

    // ── delete_nodes_for_file ─────────────────────────────────────────────────

    #[test]
    fn delete_nodes_for_file_removes_correct_nodes() {
        let engine = HermesEngine::in_memory("gq-delete").unwrap();
        let graph = make_graph(&engine);
        insert_node(&graph, "n1", "fn_a", "src/a.rs");
        insert_node(&graph, "n2", "fn_b", "src/b.rs");

        graph.delete_nodes_for_file("src/a.rs").unwrap();

        let all = graph.get_all_nodes().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].name, "fn_b");
    }

    #[test]
    fn delete_nodes_removes_associated_edges() {
        let engine = HermesEngine::in_memory("gq-delete-edges").unwrap();
        let graph = make_graph(&engine);
        let n1 = insert_node(&graph, "n1", "fn_a", "src/a.rs");
        let n2 = insert_node(&graph, "n2", "fn_b", "src/b.rs");

        let edge = Edge {
            id: "e1".to_string(),
            project_id: graph.project_id().to_string(),
            source_id: n1.id.clone(),
            target_id: n2.id.clone(),
            edge_type: EdgeType::Calls,
            weight: 1.0,
        };
        graph.add_edge(&edge).unwrap();

        graph.delete_nodes_for_file("src/a.rs").unwrap();

        // n2 still exists but has no neighbors since n1 and the edge are gone
        let neighbors = graph.get_neighbors("n2").unwrap();
        assert!(neighbors.is_empty());
    }

    // ── fts_search ───────────────────────────────────────────────────────────────

    #[test]
    fn fts_search_finds_indexed_content() {
        let engine = HermesEngine::in_memory("gq-fts").unwrap();
        let graph = make_graph(&engine);
        let node = insert_node(&graph, "n1", "alerts_handler", "src/api.rs");
        graph
            .index_fts(&node, "handles incoming alert notifications")
            .unwrap();

        let results = graph.fts_search("\"alert\"", 10).unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].0.id, "n1");
    }

    #[test]
    fn fts_search_returns_empty_for_no_match() {
        let engine = HermesEngine::in_memory("gq-fts-empty").unwrap();
        let graph = make_graph(&engine);
        let node = insert_node(&graph, "n1", "handler", "src/api.rs");
        graph.index_fts(&node, "something completely different").unwrap();

        let results = graph.fts_search("\"xyznonexistent\"", 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn fts_search_respects_limit() {
        let engine = HermesEngine::in_memory("gq-fts-limit").unwrap();
        let graph = make_graph(&engine);

        for i in 0..5 {
            let node = insert_node(
                &graph,
                &format!("n{i}"),
                &format!("handler_{i}"),
                "src/api.rs",
            );
            graph
                .index_fts(&node, "shared keyword present in content")
                .unwrap();
        }

        let results = graph.fts_search("\"shared\"", 3).unwrap();
        assert!(results.len() <= 3);
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
        content_hash: row.get(8)?,
    })
}
