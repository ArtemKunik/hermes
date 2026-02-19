package hermes.search

import org.junit.jupiter.api.Test
import org.junit.jupiter.api.Assertions.*

class SearchEngineTest {

    @Test
    fun `estimate tokens word count based`() {
        val tokens = estimateTokens("hello world foo bar")
        assertEquals(6L, tokens)
    }

    @Test
    fun `estimate tokens empty`() {
        assertEquals(0L, estimateTokens(""))
    }
}
