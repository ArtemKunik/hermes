// tools/hermes-engine/src/graph_ops.rs
use crate::graph::KnowledgeGraph;
use crate::graph_types::{Edge, EdgeType, Node, NodeType};
use anyhow::Result;
use rusqlite::params;

/// Split camelCase and snake_case identifiers into individual words.
/// e.g. "fetchExchangeRate" => ["fetch", "exchange", "rate"]
///      "get_user_data"      => ["get", "user", "data"]
pub fn split_identifier(ident: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();

    for ch in ident.chars() {
        if ch == '_' {
            if !current.is_empty() {
                words.push(current.clone());
                current.clear();
            }
        } else if ch.is_uppercase() {
            if !current.is_empty() {
                words.push(current.clone());
                current.clear();
            }
            current.push(ch.to_ascii_lowercase());
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

/// Enrich content for FTS indexing by appending camelCase/snake_case splits.
/// This allows natural-language queries like "fetch exchange rate" to match
/// identifiers like `fetchExchangeRate` in code.
pub fn enrich_content_for_fts(name: &str, content: &str) -> String {
    let mut extra = Vec::new();
    for token in name.split(|c: char| !c.is_alphanumeric() && c != '_') {
        if token.len() > 1 {
            extra.extend(split_identifier(token));
        }
    }
    for token in content.split_whitespace() {
        let cleaned: String = token.chars().filter(|c| c.is_alphanumeric() || *c == '_').collect();
        if cleaned.len() > 1 {
            extra.extend(split_identifier(&cleaned));
        }
    }
    if extra.is_empty() {
        return content.to_string();
    }
    format!("{}\n{}", content, extra.join(" "))
}

impl KnowledgeGraph {
    pub fn get_neighbors(&self, node_id: &str) -> Result<Vec<(Edge, Node)>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT e.id, e.project_id, e.source_id, e.target_id, e.edge_type, e.weight,
                        n.id, n.project_id, n.name, n.node_type, n.file_path, n.start_line, n.end_line, n.summary, n.content_tokens, n.object_type
                 FROM edges e
                 JOIN nodes n ON n.id = CASE WHEN e.source_id = ?1 THEN e.target_id ELSE e.source_id END
                 WHERE (e.source_id = ?1 OR e.target_id = ?1) AND e.project_id = ?2",
            )?;
            let rows = stmt
                .query_map(params![node_id, self.project_id()], |row| {
                    Ok((
                        Edge {
                            id: row.get(0)?,
                            project_id: row.get(1)?,
                            source_id: row.get(2)?,
                            target_id: row.get(3)?,
                            edge_type: EdgeType::parse_str(&row.get::<_, String>(4)?),
                            weight: row.get(5)?,
                        },
                        Node {
                            id: row.get(6)?,
                            project_id: row.get(7)?,
                            name: row.get(8)?,
                            node_type: NodeType::parse_str(&row.get::<_, String>(9)?),
                            file_path: row.get(10)?,
                            start_line: row.get(11)?,
                            end_line: row.get(12)?,
                            summary: row.get(13)?,
                            content_hash: None,
                            content_tokens: row.get::<_, Option<i64>>(14)?.map(|v| v as u64),
                            object_type: row.get(15)?,
                        },
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    pub fn index_fts(&self, node: &Node, content: &str) -> Result<()> {
        self.with_conn(|conn| {
            let enriched = enrich_content_for_fts(&node.name, content);
            conn.execute(
                "INSERT OR REPLACE INTO fts_content (node_id, project_id, name, content, file_path)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![node.id, node.project_id, node.name, enriched, node.file_path,],
            )?;
            Ok(())
        })
    }

    /// Hard-delete a node and all associated index data.
    pub fn delete_node(&self, node_id: &str) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "DELETE FROM fts_content   WHERE node_id = ?1",
                params![node_id],
            )?;
            conn.execute(
                "DELETE FROM pointer_cache WHERE node_id = ?1",
                params![node_id],
            )?;
            conn.execute(
                "DELETE FROM weight_index  WHERE node_id = ?1",
                params![node_id],
            )?;
            conn.execute(
                "DELETE FROM edges WHERE source_id = ?1 OR target_id = ?1",
                params![node_id],
            )?;
            conn.execute("DELETE FROM nodes WHERE id = ?1", params![node_id])?;
            Ok(())
        })
    }
}
