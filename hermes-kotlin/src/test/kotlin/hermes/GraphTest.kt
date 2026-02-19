package hermes

import org.junit.jupiter.api.Test
import org.junit.jupiter.api.Assertions.*

class GraphTest {

    private fun makeGraph(): KnowledgeGraph {
        val engine = HermesEngine.inMemory("test-graph")
        return KnowledgeGraph(engine.db, engine.projectId)
    }

    private fun sampleNode(projectId: String) = Node(
        id = "node-1",
        projectId = projectId,
        name = "my_function",
        nodeType = NodeType.FUNCTION,
        filePath = "src/lib.rs",
        startLine = 10,
        endLine = 20,
        summary = "Does something",
        contentHash = "abc123"
    )

    @Test
    fun `add and get node`() {
        val graph = makeGraph()
        val node = sampleNode(graph.projectId)
        graph.addNode(node)
        val retrieved = graph.getNode("node-1")
        assertNotNull(retrieved)
        assertEquals("my_function", retrieved!!.name)
        assertEquals(NodeType.FUNCTION, retrieved.nodeType)
    }

    @Test
    fun `get non-existent node returns null`() {
        val graph = makeGraph()
        assertNull(graph.getNode("nonexistent"))
    }

    @Test
    fun `add and get edge`() {
        val graph = makeGraph()
        val n1 = sampleNode(graph.projectId).copy(id = "n1")
        val n2 = sampleNode(graph.projectId).copy(id = "n2", name = "other_function")
        graph.addNode(n1)
        graph.addNode(n2)

        val edge = Edge(
            id = "e1",
            projectId = graph.projectId,
            sourceId = "n1",
            targetId = "n2",
            edgeType = EdgeType.CALLS,
            weight = 1.0
        )
        graph.addEdge(edge)

        val neighbors = graph.getNeighbors("n1")
        assertEquals(1, neighbors.size)
        assertEquals("n2", neighbors[0].second.id)
    }

    @Test
    fun `node type roundtrip`() {
        for (type in NodeType.entries) {
            assertEquals(type, NodeType.parse(type.value))
        }
    }

    @Test
    fun `edge type roundtrip`() {
        for (type in EdgeType.entries) {
            assertEquals(type, EdgeType.parse(type.value))
        }
    }
}
