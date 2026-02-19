package hermes

import org.junit.jupiter.api.Test
import org.junit.jupiter.api.Assertions.*

class HermesEngineTest {

    @Test
    fun `create in-memory engine`() {
        val engine = HermesEngine.inMemory("test-project")
        assertEquals("test-project", engine.projectId)
    }

    @Test
    fun `search cache starts empty`() {
        val engine = HermesEngine.inMemory("test-cache")
        assertTrue(engine.searchCache.isEmpty())
    }

    @Test
    fun `invalidate clears cache`() {
        val engine = HermesEngine.inMemory("test-inv")
        engine.searchCache["key"] = PointerResponse.build(emptyList(), 0) to java.time.Instant.now()
        assertFalse(engine.searchCache.isEmpty())
        engine.invalidateSearchCache()
        assertTrue(engine.searchCache.isEmpty())
    }
}
