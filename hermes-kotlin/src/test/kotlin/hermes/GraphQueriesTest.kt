package hermes

import org.junit.jupiter.api.Test
import org.junit.jupiter.api.Assertions.*

class GraphQueriesTest {

    private fun makeGraph(): KnowledgeGraph {
        val engine = HermesEngine.inMemory("test-gq")
        return KnowledgeGraph(engine.db, engine.projectId)
    }

    private fun insertNode(graph: KnowledgeGraph, id: String, name: String, filePath: String): Node {
        val node = Node(
            id = id,
            projectId = graph.projectId,
            name = name,
            nodeType = NodeType.FUNCTION,
            filePath = filePath,
            startLine = 1,
            endLine = 10,
            summary = null,
            contentHash = null
        )
        graph.addNode(node)
        return node
    }

    @Test
    fun `literal search prefix match`() {
        val graph = makeGraph()
        insertNode(graph, "n1", "fetch_alerts", "src/api.rs")
        insertNode(graph, "n2", "process_alerts", "src/api.rs")
        val results = graph.literalSearchByName("fetch")
        assertEquals(1, results.size)
        assertEquals("fetch_alerts", results[0].name)
    }

    @Test
    fun `literal search contains fallback`() {
        val graph = makeGraph()
        insertNode(graph, "n1", "fetch_alerts_handler", "src/api.rs")
        val results = graph.literalSearchByName("alerts")
        assertTrue(results.isNotEmpty())
        assertEquals("fetch_alerts_handler", results[0].name)
    }

    @Test
    fun `literal search is case insensitive`() {
        val graph = makeGraph()
        insertNode(graph, "n1", "HandleRequest", "src/server.rs")
        val results = graph.literalSearchByName("handlerequest")
        assertEquals(1, results.size)
    }

    @Test
    fun `get all nodes empty`() {
        val graph = makeGraph()
        assertTrue(graph.getAllNodes().isEmpty())
    }

    @Test
    fun `get all nodes returns inserted`() {
        val graph = makeGraph()
        insertNode(graph, "n1", "alpha", "src/a.rs")
        insertNode(graph, "n2", "beta", "src/b.rs")
        assertEquals(2, graph.getAllNodes().size)
    }

    @Test
    fun `fts search finds indexed content`() {
        val graph = makeGraph()
        val node = insertNode(graph, "n1", "alerts_handler", "src/api.rs")
        graph.indexFts(node, "handles incoming alert notifications")
        val results = graph.ftsSearch("\"alert\"", 10)
        assertTrue(results.isNotEmpty())
        assertEquals("n1", results[0].first.id)
    }

    @Test
    fun `delete nodes for file removes correct nodes`() {
        val graph = makeGraph()
        insertNode(graph, "n1", "fn_a", "src/a.rs")
        insertNode(graph, "n2", "fn_b", "src/b.rs")
        graph.deleteNodesForFile("src/a.rs")
        val all = graph.getAllNodes()
        assertEquals(1, all.size)
        assertEquals("fn_b", all[0].name)
    }
}
