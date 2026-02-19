package hermes

/**
 * Extension functions on KnowledgeGraph for queries (mirrors graph_queries.rs).
 */

fun KnowledgeGraph.literalSearchByName(query: String): List<Node> {
    val queryLower = query.lowercase()
    val allNodes = mutableListOf<Node>()

    db.prepareStatement(
        """SELECT id, project_id, name, node_type, file_path, start_line, end_line, summary, content_hash
           FROM nodes WHERE project_id = ?"""
    ).use { ps ->
        ps.setString(1, projectId)
        ps.executeQuery().use { rs ->
            while (rs.next()) {
                allNodes.add(KnowledgeGraph.nodeFromResultSet(rs))
            }
        }
    }

    // Prefer prefix matches; fall back to contains matches.
    val prefixResults = allNodes.filter { it.name.lowercase().startsWith(queryLower) }
    if (prefixResults.isNotEmpty()) return prefixResults

    return allNodes.filter { it.name.lowercase().contains(queryLower) }
}

fun KnowledgeGraph.getAllFilePaths(): Set<String> {
    val paths = mutableSetOf<String>()
    db.prepareStatement(
        """SELECT DISTINCT file_path FROM nodes
           WHERE project_id = ? AND node_type = 'file' AND file_path IS NOT NULL"""
    ).use { ps ->
        ps.setString(1, projectId)
        ps.executeQuery().use { rs ->
            while (rs.next()) {
                paths.add(rs.getString(1))
            }
        }
    }
    return paths
}

fun KnowledgeGraph.deleteNodesForFile(filePath: String) {
    db.prepareStatement(
        """DELETE FROM fts_content WHERE node_id IN
           (SELECT id FROM nodes WHERE file_path = ? AND project_id = ?)"""
    ).use { ps ->
        ps.setString(1, filePath)
        ps.setString(2, projectId)
        ps.executeUpdate()
    }
    db.prepareStatement(
        """DELETE FROM edges WHERE
           source_id IN (SELECT id FROM nodes WHERE file_path = ? AND project_id = ?)
           OR target_id IN (SELECT id FROM nodes WHERE file_path = ? AND project_id = ?)"""
    ).use { ps ->
        ps.setString(1, filePath)
        ps.setString(2, projectId)
        ps.setString(3, filePath)
        ps.setString(4, projectId)
        ps.executeUpdate()
    }
    db.prepareStatement(
        "DELETE FROM nodes WHERE file_path = ? AND project_id = ?"
    ).use { ps ->
        ps.setString(1, filePath)
        ps.setString(2, projectId)
        ps.executeUpdate()
    }
}

fun KnowledgeGraph.getAllNodes(): List<Node> {
    val nodes = mutableListOf<Node>()
    db.prepareStatement(
        """SELECT id, project_id, name, node_type, file_path, start_line, end_line, summary, content_hash
           FROM nodes WHERE project_id = ?"""
    ).use { ps ->
        ps.setString(1, projectId)
        ps.executeQuery().use { rs ->
            while (rs.next()) {
                nodes.add(KnowledgeGraph.nodeFromResultSet(rs))
            }
        }
    }
    return nodes
}

fun KnowledgeGraph.ftsSearch(query: String, limit: Int): List<Pair<Node, Double>> {
    val results = mutableListOf<Pair<Node, Double>>()
    db.prepareStatement(
        """SELECT n.id, n.project_id, n.name, n.node_type, n.file_path, n.start_line, n.end_line, n.summary, n.content_hash,
                  bm25(fts_content) as rank
           FROM fts_content f
           JOIN nodes n ON n.id = f.node_id
           WHERE fts_content MATCH ? AND f.project_id = ?
           ORDER BY rank
           LIMIT ?"""
    ).use { ps ->
        ps.setString(1, query)
        ps.setString(2, projectId)
        ps.setInt(3, limit)
        ps.executeQuery().use { rs ->
            while (rs.next()) {
                val node = KnowledgeGraph.nodeFromResultSet(rs)
                val rank = rs.getDouble(10)
                results.add(node to rank)
            }
        }
    }
    return results
}
