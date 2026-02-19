package hermes.ingestion

import hermes.NodeType
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.Assertions.*
import java.nio.file.Path

class ChunkerTest {

    @Test
    fun `chunk rust function`() {
        val code = "pub fn hello(name: &str) -> String {\n    format!(\"Hello {name}\")\n}\n"
        val chunks = chunkFile(Path.of("test.rs"), code)
        assertEquals(1, chunks.size)
        assertEquals("hello", chunks[0].name)
        assertEquals(NodeType.FUNCTION, chunks[0].nodeType)
    }

    @Test
    fun `chunk rust struct`() {
        val code = "pub struct Config {\n    pub port: u16,\n}\n"
        val chunks = chunkFile(Path.of("test.rs"), code)
        assertEquals(1, chunks.size)
        assertEquals("Config", chunks[0].name)
        assertEquals(NodeType.STRUCT, chunks[0].nodeType)
    }

    @Test
    fun `chunk markdown sections`() {
        val md = "# Title\nIntro\n## Section A\nContent A\n## Section B\nContent B\n"
        val chunks = chunkFile(Path.of("test.md"), md)
        assertEquals(3, chunks.size)
        assertEquals("Title", chunks[0].name)
    }
}
