package hermes.search

import org.junit.jupiter.api.Test
import org.junit.jupiter.api.Assertions.*

class FtsTest {

    @Test
    fun `filters fts operators`() {
        val words = extractWords("NOT main AND test OR foo")
        assertFalse(words.any { it.equals("NOT", ignoreCase = true) })
        assertFalse(words.any { it.equals("AND", ignoreCase = true) })
        assertFalse(words.any { it.equals("OR", ignoreCase = true) })
        assertTrue(words.contains("main"))
        assertTrue(words.contains("test"))
        assertTrue(words.contains("foo"))
    }

    @Test
    fun `truncates to ten words`() {
        val longQuery = "a b c d e f g h i j k l m n"
        val words = extractWords(longQuery)
        assertEquals(10, words.size)
    }

    @Test
    fun `ignores punctuation like slashes`() {
        val tokens = extractWords("/api/alerts handler")
        assertEquals(listOf("api", "alerts", "handler"), tokens)
    }

    @Test
    fun `bm25 normalization`() {
        assertTrue(normalizeBm25Score(-5.0) > 0.5)
        assertTrue(normalizeBm25Score(-10.0) > normalizeBm25Score(-5.0))
        assertTrue(normalizeBm25Score(0.0) < 0.6)
    }
}
