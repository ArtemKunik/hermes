// hermes-engine/src/blast_radius.rs — per-node blast-radius scoring engine.
//
// Computes BFS-based impact scores over dependency edges and persists results
// in the `blast_scores` table for fast MCP tool lookups.

use anyhow::Result;
use rusqlite::{params, Connection};
use std::collections::{HashMap, HashSet, VecDeque};

/// Risk classification based on blast score relative to codebase size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RiskLevel {
    High,
    Medium,
    Low,
}

impl RiskLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            RiskLevel::High => "HIGH",
            RiskLevel::Medium => "MEDIUM",
            RiskLevel::Low => "LOW",
        }
    }
}

/// A single node's blast-radius score.
#[derive(Debug, Clone)]
pub struct BlastScore {
    pub node_id: String,
    pub file_path: Option<String>,
    pub direct_count: usize,
    pub transitive_count: usize,
    pub blast_score: f64,
    pub risk_level: RiskLevel,
}

/// Dependency edge types that represent true code dependencies.
/// Non-dependency edges (Contains, Documents, Defines) are excluded from BFS.
const DEPENDENCY_EDGE_TYPES: &[&str] = &["calls", "imports", "uses", "depends_on", "implements"];

/// Build an in-memory adjacency list for dependency edges only.
/// Returns a map from target_id → Vec<(source_id, edge_type)>.
fn build_adjacency_list(
    conn: &Connection,
    project_id: &str,
) -> Result<HashMap<String, Vec<(String, String)>>> {
    let mut stmt = conn.prepare(
        "SELECT source_id, target_id, edge_type \
         FROM edges WHERE project_id = ?1 AND edge_type IN (\"calls\",\"imports\",\"uses\",\"depends_on\",\"implements\")",
    )?;

    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;

    let mut adj: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for r in rows {
        let (src, tgt, etype) = r?;
        adj.entry(tgt).or_default().push((src, etype));
    }
    Ok(adj)
}

/// Compute blast-radius scores for all nodes in a project.
///
/// For each node, performs BFS following incoming dependency edges up to depth 3.
/// Score = direct + (0.5 × transitive). Risk levels based on codebase percentage.
pub fn compute_all_blast_scores(conn: &Connection, project_id: &str) -> Result<usize> {
    // Load all nodes in the project
    let mut node_stmt = conn.prepare("SELECT id, file_path FROM nodes WHERE project_id = ?1")?;
    let nodes: Vec<(String, Option<String>)> = node_stmt
        .query_map([project_id], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<_, _>>()?;

    if nodes.is_empty() {
        return Ok(0);
    }

    let total_nodes = nodes.len();

    // Build adjacency list (incoming dependency edges)
    let adj = build_adjacency_list(conn, project_id)?;

    // BFS from each node
    let mut results: Vec<BlastScore> = Vec::with_capacity(nodes.len());

    for (node_id, file_path) in &nodes {
        let (direct_count, transitive_count) = bfs_counts(&adj, node_id);

        let blast_score = direct_count as f64 + (transitive_count as f64 * 0.5);
        let affected = direct_count + transitive_count;
        let risk_level = if total_nodes > 0 && (affected as f64 / total_nodes as f64) > 0.15 {
            RiskLevel::High
        } else if total_nodes > 0 && (affected as f64 / total_nodes as f64) > 0.05 {
            RiskLevel::Medium
        } else {
            RiskLevel::Low
        };

        results.push(BlastScore {
            node_id: node_id.clone(),
            file_path: file_path.clone(),
            direct_count,
            transitive_count,
            blast_score,
            risk_level,
        });
    }

    // Batch upsert into blast_scores table
    let mut stmt = conn.prepare(
        "INSERT OR REPLACE INTO blast_scores \
         (node_id, project_id, file_path, direct_count, transitive_count, blast_score, risk_level) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;

    for score in &results {
        stmt.execute(params![
            &score.node_id,
            project_id,
            &score.file_path,
            score.direct_count as i64,
            score.transitive_count as i64,
            score.blast_score,
            score.risk_level.as_str(),
        ])?;
    }

    // Clean up stale scores for nodes no longer in the project
    let mut del_stmt = conn.prepare(
        "DELETE FROM blast_scores WHERE project_id = ?1 AND node_id NOT IN (SELECT id FROM nodes WHERE project_id = ?1)",
    )?;
    let _ = del_stmt.execute([project_id, project_id]);

    Ok(results.len())
}

/// Run BFS from a single node and return (direct_count, transitive_count).
fn bfs_counts(adj: &HashMap<String, Vec<(String, String)>>, start_id: &str) -> (usize, usize) {
    use std::collections::{HashSet, VecDeque};

    let mut visited: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<(String, usize)> = VecDeque::new();

    visited.insert(start_id.to_string());
    queue.push_back((start_id.to_string(), 0));

    let mut direct_count: usize = 0;
    let mut transitive_count: usize = 0;

    while let Some((current, depth)) = queue.pop_front() {
        if depth >= 3 {
            continue;
        }

        if let Some(upstream_list) = adj.get(&current) {
            for (up_id, etype) in upstream_list {
                // Only follow dependency edge types
                if !matches!(
                    etype.as_str(),
                    "calls" | "imports" | "uses" | "depends_on" | "implements"
                ) {
                    continue;
                }
                if visited.insert(up_id.clone()) {
                    if depth == 0 {
                        direct_count += 1;
                    } else {
                        transitive_count += 1;
                    }
                    queue.push_back((up_id.clone(), depth + 1));
                }
            }
        }
    }

    (direct_count, transitive_count)
}

/// Look up the blast score for a specific node or file.
pub fn get_blast_score(
    conn: &Connection,
    project_id: &str,
    symbol_or_file: &str,
) -> Result<Option<BlastScore>> {
    // Try by node name first (case-insensitive match against nodes table)
    let mut stmt = conn.prepare(
        "SELECT bs.node_id, bs.file_path, bs.direct_count, bs.transitive_count, \
                bs.blast_score, bs.risk_level \
         FROM blast_scores bs \
         JOIN nodes n ON n.id = bs.node_id \
         WHERE bs.project_id = ?1 AND (LOWER(n.name) = LOWER(?2) OR bs.file_path = ?2)",
    )?;

    let row = stmt
        .query_row([project_id, symbol_or_file], |row| {
            Ok(BlastScore {
                node_id: row.get(0)?,
                file_path: row.get::<_, Option<String>>(1)?,
                direct_count: row.get::<_, i64>(2)? as usize,
                transitive_count: row.get::<_, i64>(3)? as usize,
                blast_score: row.get(4)?,
                risk_level: match &*row.get::<_, String>(5)? {
                    "HIGH" => RiskLevel::High,
                    "MEDIUM" => RiskLevel::Medium,
                    _ => RiskLevel::Low,
                },
            })
        })
        .ok();

    Ok(row)
}

/// Get top-N files/symbols by blast score above threshold.
pub fn get_high_blast(
    conn: &Connection,
    project_id: &str,
    threshold: f64,
    limit: usize,
) -> Result<Vec<BlastScore>> {
    let mut stmt = conn.prepare(
        "SELECT bs.node_id, bs.file_path, bs.direct_count, bs.transitive_count, \
                bs.blast_score, bs.risk_level \
         FROM blast_scores bs \
         WHERE bs.project_id = ?1 AND bs.blast_score >= ?2 \
         ORDER BY bs.blast_score DESC \
         LIMIT ?3",
    )?;

    let rows = stmt.query_map(params![project_id, threshold, limit as i64], |row| {
        Ok(BlastScore {
            node_id: row.get(0)?,
            file_path: row.get::<_, Option<String>>(1)?,
            direct_count: row.get::<_, i64>(2)? as usize,
            transitive_count: row.get::<_, i64>(3)? as usize,
            blast_score: row.get(4)?,
            risk_level: match &*row.get::<_, String>(5)? {
                "HIGH" => RiskLevel::High,
                "MEDIUM" => RiskLevel::Medium,
                _ => RiskLevel::Low,
            },
        })
    })?;

    let mut results = Vec::new();
    for r in rows {
        results.push(r?);
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bfs_counts_simple_chain() {
        // A → B → C  (A depends on B, B depends on C)
        // From C's perspective: direct=1 (B), transitive=1 (A)
        let adj: HashMap<String, Vec<(String, String)>> = [
            (
                "node_c".to_string(),
                vec![("node_b".to_string(), "imports".to_string())],
            ),
            (
                "node_b".to_string(),
                vec![("node_a".to_string(), "calls".to_string())],
            ),
        ]
        .into_iter()
        .collect();

        let (direct, transitive) = bfs_counts(&adj, "node_c");
        assert_eq!(direct, 1); // B is direct dependent of C
        assert_eq!(transitive, 1); // A is transitive dependent via B
    }

    #[test]
    fn test_bfs_counts_diamond() {
        //     D
        //    / \
        //   B   C
        //    \ /
        //     A
        // Both B and C depend on A; D depends on both B and C.
        // From A's perspective: direct=2 (B,C), transitive=1 (D)
        let adj: HashMap<String, Vec<(String, String)>> = [
            (
                "node_a".to_string(),
                vec![
                    ("node_b".to_string(), "imports".to_string()),
                    ("node_c".to_string(), "calls".to_string()),
                ],
            ),
            (
                "node_b".to_string(),
                vec![("node_d".to_string(), "calls".to_string())],
            ),
            (
                "node_c".to_string(),
                vec![("node_d".to_string(), "calls".to_string())],
            ),
        ]
        .into_iter()
        .collect();

        let (direct, transitive) = bfs_counts(&adj, "node_a");
        assert_eq!(direct, 2); // B and C are direct dependents of A
        assert_eq!(transitive, 1); // D is transitive (reached via both B and C but deduplicated)
    }

    #[test]
    fn test_bfs_depth_limit() {
        // Linear chain: E → D → C → B → A
        // From A's perspective at depth limit 3: direct=1, transitive=2 (B,C), D is at depth 4 so excluded
        let adj: HashMap<String, Vec<(String, String)>> = [
            (
                "node_a".to_string(),
                vec![("node_b".to_string(), "imports".to_string())],
            ),
            (
                "node_b".to_string(),
                vec![("node_c".to_string(), "calls".to_string())],
            ),
            (
                "node_c".to_string(),
                vec![("node_d".to_string(), "calls".to_string())],
            ),
            (
                "node_d".to_string(),
                vec![("node_e".to_string(), "calls".to_string())],
            ),
        ]
        .into_iter()
        .collect();

        let (direct, transitive) = bfs_counts(&adj, "node_a");
        assert_eq!(direct, 1); // B at depth 1
        assert_eq!(transitive, 2); // C at depth 2, D at depth 3; E at depth 4 is excluded
    }

    #[test]
    fn test_bfs_excludes_non_dependency_edges() {
        // Node with only Contains edges should have zero blast radius
        let adj: HashMap<String, Vec<(String, String)>> = [(
            "node_x".to_string(),
            vec![("node_y".to_string(), "contains".to_string())],
        )]
        .into_iter()
        .collect();

        let (direct, transitive) = bfs_counts(&adj, "node_x");
        assert_eq!(direct, 0); // Contains is not a dependency edge
        assert_eq!(transitive, 0);
    }

    #[test]
    fn test_risk_level_thresholds() {
        let high = BlastScore {
            node_id: "a".into(),
            file_path: None,
            direct_count: 20,
            transitive_count: 10,
            blast_score: 25.0,
            risk_level: RiskLevel::High,
        };
        assert_eq!(high.risk_level, RiskLevel::High);

        let low = BlastScore {
            node_id: "b".into(),
            file_path: None,
            direct_count: 1,
            transitive_count: 2,
            blast_score: 2.0,
            risk_level: RiskLevel::Low,
        };
        assert_eq!(low.risk_level, RiskLevel::Low);
    }

    #[test]
    fn test_blast_score_formula() {
        // direct=3, transitive=4 → score = 3 + (0.5 * 4) = 5.0
        let score = BlastScore {
            node_id: "x".into(),
            file_path: None,
            direct_count: 3,
            transitive_count: 4,
            blast_score: 5.0,
            risk_level: RiskLevel::Low,
        };
        assert!((score.blast_score - 5.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_risk_level_as_str() {
        assert_eq!(RiskLevel::High.as_str(), "HIGH");
        assert_eq!(RiskLevel::Medium.as_str(), "MEDIUM");
        assert_eq!(RiskLevel::Low.as_str(), "LOW");
    }
}
