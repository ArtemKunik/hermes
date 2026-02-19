package hermes

import kotlinx.serialization.Serializable
import kotlinx.serialization.SerialName
import java.sql.Connection
import java.time.ZonedDateTime
import java.time.format.DateTimeFormatter
import java.util.UUID

// ── Node & Edge types ─────────────────────────────────────────────────────

@Serializable
enum class NodeType(val value: String) {
    @SerialName("file") FILE("file"),
    @SerialName("module") MODULE("module"),
    @SerialName("function") FUNCTION("function"),
    @SerialName("struct") STRUCT("struct"),
    @SerialName("impl") IMPL("impl"),
    @SerialName("trait") TRAIT("trait"),
    @SerialName("enum") ENUM("enum"),
    @SerialName("concept") CONCEPT("concept"),
    @SerialName("document") DOCUMENT("document");

    companion object {
        fun parse(s: String): NodeType = entries.find { it.value == s } ?: CONCEPT
    }
}

@Serializable
enum class EdgeType(val value: String) {
    @SerialName("calls") CALLS("calls"),
    @SerialName("imports") IMPORTS("imports"),
    @SerialName("implements") IMPLEMENTS("implements"),
    @SerialName("depends_on") DEPENDS_ON("depends_on"),
    @SerialName("contains") CONTAINS("contains"),
    @SerialName("documents") DOCUMENTS("documents");

    companion object {
        fun parse(s: String): EdgeType = entries.find { it.value == s } ?: DEPENDS_ON
    }
}

// ── Data classes ──────────────────────────────────────────────────────────

@Serializable
data class Node(
    val id: String,
    val projectId: String,
    val name: String,
    val nodeType: NodeType,
    val filePath: String? = null,
    val startLine: Long? = null,
    val endLine: Long? = null,
    val summary: String? = null,
    val contentHash: String? = null
)

@Serializable
data class Edge(
    val id: String,
    val projectId: String,
    val sourceId: String,
    val targetId: String,
    val edgeType: EdgeType,
    val weight: Double = 1.0
)

// ── KnowledgeGraph ───────────────────────────────────────────────────────

class KnowledgeGraph(
    val db: Connection,
    val projectId: String
) {
    fun addNode(node: Node) {
        val now = ZonedDateTime.now().format(DateTimeFormatter.ISO_OFFSET_DATE_TIME)
        db.prepareStatement(
            """INSERT OR REPLACE INTO nodes
               (id, project_id, name, node_type, file_path, start_line, end_line, summary, content_hash, updated_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"""
        ).use { ps ->
            ps.setString(1, node.id)
            ps.setString(2, node.projectId)
            ps.setString(3, node.name)
            ps.setString(4, node.nodeType.value)
            ps.setString(5, node.filePath)
            setNullableLong(ps, 6, node.startLine)
            setNullableLong(ps, 7, node.endLine)
            ps.setString(8, node.summary)
            ps.setString(9, node.contentHash)
            ps.setString(10, now)
            ps.executeUpdate()
        }
    }

    fun getNode(nodeId: String): Node? {
        db.prepareStatement(
            """SELECT id, project_id, name, node_type, file_path, start_line, end_line, summary, content_hash
               FROM nodes WHERE id = ? AND project_id = ?"""
        ).use { ps ->
            ps.setString(1, nodeId)
            ps.setString(2, projectId)
            ps.executeQuery().use { rs ->
                if (!rs.next()) return null
                return nodeFromResultSet(rs)
            }
        }
    }

    fun addEdge(edge: Edge) {
        db.prepareStatement(
            """INSERT OR IGNORE INTO edges (id, project_id, source_id, target_id, edge_type, weight)
               VALUES (?, ?, ?, ?, ?, ?)"""
        ).use { ps ->
            ps.setString(1, edge.id)
            ps.setString(2, edge.projectId)
            ps.setString(3, edge.sourceId)
            ps.setString(4, edge.targetId)
            ps.setString(5, edge.edgeType.value)
            ps.setDouble(6, edge.weight)
            ps.executeUpdate()
        }
    }

    fun getNeighbors(nodeId: String): List<Pair<Edge, Node>> {
        val results = mutableListOf<Pair<Edge, Node>>()
        db.prepareStatement(
            """SELECT e.id, e.project_id, e.source_id, e.target_id, e.edge_type, e.weight,
                      n.id, n.project_id, n.name, n.node_type, n.file_path, n.start_line, n.end_line, n.summary, n.content_hash
               FROM edges e
               JOIN nodes n ON n.id = CASE WHEN e.source_id = ? THEN e.target_id ELSE e.source_id END
               WHERE (e.source_id = ? OR e.target_id = ?) AND e.project_id = ?"""
        ).use { ps ->
            ps.setString(1, nodeId)
            ps.setString(2, nodeId)
            ps.setString(3, nodeId)
            ps.setString(4, projectId)
            ps.executeQuery().use { rs ->
                while (rs.next()) {
                    val edge = Edge(
                        id = rs.getString(1),
                        projectId = rs.getString(2),
                        sourceId = rs.getString(3),
                        targetId = rs.getString(4),
                        edgeType = EdgeType.parse(rs.getString(5)),
                        weight = rs.getDouble(6)
                    )
                    val node = Node(
                        id = rs.getString(7),
                        projectId = rs.getString(8),
                        name = rs.getString(9),
                        nodeType = NodeType.parse(rs.getString(10)),
                        filePath = rs.getString(11),
                        startLine = rs.getLong(12).takeIf { !rs.wasNull() },
                        endLine = rs.getLong(13).takeIf { !rs.wasNull() },
                        summary = rs.getString(14),
                        contentHash = rs.getString(15)
                    )
                    results.add(edge to node)
                }
            }
        }
        return results
    }

    fun indexFts(node: Node, content: String) {
        db.prepareStatement("DELETE FROM fts_content WHERE node_id = ?").use { ps ->
            ps.setString(1, node.id)
            ps.executeUpdate()
        }
        db.prepareStatement(
            """INSERT INTO fts_content (node_id, project_id, name, content, file_path)
               VALUES (?, ?, ?, ?, ?)"""
        ).use { ps ->
            ps.setString(1, node.id)
            ps.setString(2, node.projectId)
            ps.setString(3, node.name)
            ps.setString(4, content)
            ps.setString(5, node.filePath)
            ps.executeUpdate()
        }
    }

    fun createNodeBuilder(): NodeBuilder = NodeBuilder(projectId)
    fun createEdgeBuilder(): EdgeBuilder = EdgeBuilder(projectId)

    companion object {
        fun nodeFromResultSet(rs: java.sql.ResultSet): Node = Node(
            id = rs.getString(1),
            projectId = rs.getString(2),
            name = rs.getString(3),
            nodeType = NodeType.parse(rs.getString(4)),
            filePath = rs.getString(5),
            startLine = rs.getLong(6).takeIf { !rs.wasNull() },
            endLine = rs.getLong(7).takeIf { !rs.wasNull() },
            summary = rs.getString(8),
            contentHash = rs.getString(9)
        )

        private fun setNullableLong(ps: java.sql.PreparedStatement, index: Int, value: Long?) {
            if (value != null) ps.setLong(index, value)
            else ps.setNull(index, java.sql.Types.INTEGER)
        }
    }
}
