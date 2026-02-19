package hermes

import java.util.UUID

class NodeBuilder(private val projectId: String) {
    private var id: String = UUID.randomUUID().toString()
    private var name: String = ""
    private var nodeType: NodeType = NodeType.CONCEPT
    private var filePath: String? = null
    private var startLine: Long? = null
    private var endLine: Long? = null
    private var summary: String? = null
    private var contentHash: String? = null

    fun name(name: String) = apply { this.name = name }
    fun nodeType(type_: NodeType) = apply { this.nodeType = type_ }
    fun filePath(path: String) = apply { this.filePath = path }
    fun lines(start: Long, end: Long) = apply { this.startLine = start; this.endLine = end }
    fun summary(summary: String) = apply { this.summary = summary }
    fun contentHash(hash: String) = apply { this.contentHash = hash }

    fun build(): Node = Node(
        id = id,
        projectId = projectId,
        name = name,
        nodeType = nodeType,
        filePath = filePath,
        startLine = startLine,
        endLine = endLine,
        summary = summary,
        contentHash = contentHash
    )
}

class EdgeBuilder(private val projectId: String) {
    private var id: String = UUID.randomUUID().toString()
    private var sourceId: String = ""
    private var targetId: String = ""
    private var edgeType: EdgeType = EdgeType.DEPENDS_ON
    private var weight: Double = 1.0

    fun source(sourceId: String) = apply { this.sourceId = sourceId }
    fun target(targetId: String) = apply { this.targetId = targetId }
    fun edgeType(type_: EdgeType) = apply { this.edgeType = type_ }
    fun weight(weight: Double) = apply { this.weight = weight }

    fun build(): Edge = Edge(
        id = id,
        projectId = projectId,
        sourceId = sourceId,
        targetId = targetId,
        edgeType = edgeType,
        weight = weight
    )
}
