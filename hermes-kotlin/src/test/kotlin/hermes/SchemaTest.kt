package hermes

import org.junit.jupiter.api.Test
import org.junit.jupiter.api.Assertions.*

class SchemaTest {

    @Test
    fun `migrations run without error`() {
        val engine = HermesEngine.inMemory("test-schema")
        // If we got here, migrations succeeded
        assertNotNull(engine.db)
    }

    @Test
    fun `migrations are idempotent`() {
        val engine = HermesEngine.inMemory("test-idem")
        // Run migrations again on the same connection
        Schema.runMigrations(engine.db)
        assertNotNull(engine.db)
    }
}
