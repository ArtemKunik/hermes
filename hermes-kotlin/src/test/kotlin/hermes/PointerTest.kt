package hermes

import org.junit.jupiter.api.Test
import org.junit.jupiter.api.Assertions.*

class PointerTest {

    @Test
    fun `pointer token estimation`() {
        val ptr = Pointer(
            id = "abc",
            source = "src/main.rs",
            chunk = "fn main",
            lines = "1-20",
            relevance = 0.95,
            summary = "Application entry point",
            nodeType = "function"
        )
        val tokens = ptr.estimateTokenCount()
        assertTrue(tokens > 0 && tokens < 100)
    }

    @Test
    fun `pointer response calculates savings`() {
        val ptrs = listOf(
            Pointer(
                id = "1", source = "src/lib.rs", chunk = "struct Engine",
                lines = "10-30", relevance = 0.9,
                summary = "Main engine struct with configuration",
                nodeType = "struct"
            )
        )
        val resp = PointerResponse.build(ptrs, 0)
        assertTrue(resp.accounting.savingsPct > 0.0)
        assertTrue(resp.accounting.traditionalRagEstimate > resp.accounting.pointerTokens)
    }

    @Test
    fun `pointer response empty has zero savings`() {
        val resp = PointerResponse.build(emptyList(), 0)
        assertEquals(0L, resp.accounting.pointerTokens)
        assertEquals(0.0, resp.accounting.savingsPct)
    }
}
