package hermes.ingestion

import org.junit.jupiter.api.Test
import org.junit.jupiter.api.Assertions.*

class HashTrackerTest {

    @Test
    fun `hash is deterministic`() {
        val h1 = computeHash("hello world")
        val h2 = computeHash("hello world")
        assertEquals(h1, h2)
    }

    @Test
    fun `different content different hash`() {
        val h1 = computeHash("hello")
        val h2 = computeHash("world")
        assertNotEquals(h1, h2)
    }

    @Test
    fun `hash is 64 hex chars`() {
        val h = computeHash("test")
        assertEquals(64, h.length)
        assertTrue(h.all { it in "0123456789abcdef" })
    }
}
