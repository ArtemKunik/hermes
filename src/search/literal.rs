use crate::graph::KnowledgeGraph;
use crate::search::{SearchResult, SearchTier};
use anyhow::Result;

pub fn literal_search(graph: &KnowledgeGraph, query: &str) -> Result<Vec<SearchResult>> {
    let nodes = graph.literal_search_by_name(query)?;
    Ok(nodes
        .into_iter()
        .map(|node| {
            let name_lower = node.name.to_lowercase();
            let query_lower = query.to_lowercase();
            let score = if name_lower == query_lower {
                1.0
            } else if name_lower.starts_with(&query_lower) {
                0.9
            } else {
                0.7
            };
            SearchResult {
                node,
                score,
                tier: SearchTier::L0Literal,
                matched_content: None,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_graph_returns_empty() {
        let engine = crate::HermesEngine::in_memory("test-lit-empty").unwrap();
        let graph = crate::graph::KnowledgeGraph::new(engine.db().clone(), engine.project_id());
        let results = literal_search(&graph, "anything").unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn literal_search_uses_sql_index() {
        let engine = crate::HermesEngine::in_memory("test-lit").unwrap();
        let graph = crate::graph::KnowledgeGraph::new(engine.db().clone(), engine.project_id());
        graph
            .add_node(&crate::graph_types::Node {
                id: "n1".to_string(),
                project_id: engine.project_id().to_string(),
                name: "fetch_exchange_rate".to_string(),
                node_type: crate::graph_types::NodeType::Function,
                file_path: None,
                start_line: None,
                end_line: None,
                summary: None,
                content_hash: None,
                content_tokens: None,
                object_type: None,
            })
            .unwrap();
        let results = literal_search(&graph, "fetch_exchange_rate").unwrap();
        assert_eq!(results.len(), 1);
        assert!((results[0].score - 1.0).abs() < f64::EPSILON);
    }
}
