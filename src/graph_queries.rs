use crate::graph::{KnowledgeGraph, Node, NodeType};
use crate::lock_ext::LockExt;
use anyhow::Result;
use rusqlite::params;
use std::collections::HashSet;

impl KnowledgeGraph {
    pub fn literal_search_by_name(&self, query: &str) -> Result<Vec<Node>> {
        let query_lower = query.to_lowercase();
        if query_lower.is_ascii() {
            let prefix_results = self.literal_search_ascii(&query_lower, true)?;
            if !prefix_results.is_empty() {
                return Ok(prefix_results);
            }

            let contains_results = self.literal_search_ascii(&query_lower, false)?;
            if !contains_results.is_empty() {
                return Ok(contains_results);
            }
        }

        self.literal_search_unicode_fallback(&query_lower)
    }

    fn literal_search_ascii(&self, query_lower: &str, prefix_only: bool) -> Result<Vec<Node>> {
        let conn = self.db().lock_ctx("graph_queries")?;
        let pattern = if prefix_only {
            format!("{query_lower}%")
        } else {
            format!("%{query_lower}%")
        };
        let mut stmt = conn.prepare(
            "SELECT id, project_id, name, node_type, file_path, start_line, end_line, summary, content_hash, content_tokens, object_type
             FROM nodes
             WHERE project_id = ?1 AND LOWER(name) LIKE ?2
             ORDER BY LENGTH(name), name
             LIMIT 100",
        )?;
        let rows = stmt
            .query_map(params![self.project_id(), pattern], node_from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn literal_search_unicode_fallback(&self, query_lower: &str) -> Result<Vec<Node>> {
        let conn = self.db().lock_ctx("graph_queries")?;
        let mut stmt = conn.prepare(
            "SELECT id, project_id, name, node_type, file_path, start_line, end_line, summary, content_hash, content_tokens, object_type
             FROM nodes WHERE project_id = ?1",
        )?;
        let all_nodes: Vec<Node> = stmt
            .query_map(params![self.project_id()], node_from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let prefix_results: Vec<Node> = all_nodes
            .iter()
            .filter(|n| n.name.to_lowercase().starts_with(query_lower))
            .cloned()
            .collect();
        if !prefix_results.is_empty() {
            return Ok(prefix_results);
        }

        let results: Vec<Node> = all_nodes
            .into_iter()
            .filter(|n| n.name.to_lowercase().contains(query_lower))
            .collect();
        Ok(results)
    }

    pub fn get_all_file_paths(&self) -> Result<HashSet<String>> {
        let conn = self.db().lock_ctx("graph_queries")?;
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
        let conn = self.db().lock_ctx("graph_queries")?;
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
        let conn = self.db().lock_ctx("graph_queries")?;
        let mut stmt = conn.prepare(
            "SELECT id, project_id, name, node_type, file_path, start_line, end_line, summary, content_hash, content_tokens, object_type
             FROM nodes WHERE project_id = ?1",
        )?;
        let rows = stmt
            .query_map(params![self.project_id()], node_from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn fts_search(&self, query: &str, limit: usize) -> Result<Vec<(Node, f64)>> {
        let conn = self.db().lock_ctx("graph_queries")?;
        let mut stmt = conn.prepare(
            "SELECT n.id, n.project_id, n.name, n.node_type, n.file_path, n.start_line, n.end_line, n.summary, n.content_hash, n.content_tokens, n.object_type,
                    bm25(fts_content) as rank
             FROM fts_content f
             JOIN nodes n ON n.id = f.node_id
             WHERE fts_content MATCH ?1 AND f.project_id = ?2
             ORDER BY rank
             LIMIT ?3",
        )?;
        let rows = stmt
            .query_map(params![query, self.project_id(), limit as i64], |row| {
                Ok((node_from_row(row)?, row.get::<_, f64>(11)?))
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
            content_tokens: None,
            object_type: None,
        };
        graph.add_node(&node).unwrap();
        node
    }

    #[test]
    fn get_all_file_paths_only_returns_file_type_nodes() {
        let engine = HermesEngine::in_memory("gq-filepaths").unwrap();
        let graph = make_graph(&engine);

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
            content_tokens: None,
            object_type: None,
        };
        graph.add_node(&file_node).unwrap();
        insert_node(&graph, "fn-1", "some_fn", "src/main.rs");

        let paths = graph.get_all_file_paths().unwrap();
        assert_eq!(paths.len(), 1);
        assert!(paths.contains("src/main.rs"));
    }

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

        graph.add_edge(&Edge {
            id: "e1".to_string(),
            project_id: graph.project_id().to_string(),
            source_id: n1.id.clone(),
            target_id: n2.id.clone(),
            edge_type: EdgeType::Calls,
            weight: 1.0,
        }).unwrap();

        graph.delete_nodes_for_file("src/a.rs").unwrap();
        let neighbors = graph.get_neighbors("n2").unwrap();
        assert!(neighbors.is_empty());
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
        content_tokens: row.get::<_, Option<i64>>(9)?.map(|v| v as u64),
        object_type: row.get(10)?,
    })
}
