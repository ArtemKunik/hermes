package hermes

import org.junit.jupiter.api.Test
import org.junit.jupiter.api.Assertions.*

class TemporalTest {

    @Test
    fun `add and retrieve fact`() {
        val engine = HermesEngine.inMemory("test-temp")
        val store = TemporalStore(engine.db, "test-temp")
        val id = store.addFact(
            factType = FactType.ARCHITECTURE,
            content = "Backend uses Axum + Tokio",
            sourceReference = "initial setup"
        )
        val facts = store.getActiveFacts()
        assertEquals(1, facts.size)
        assertEquals(id, facts[0].id)
        assertEquals("Backend uses Axum + Tokio", facts[0].content)
        assertNull(facts[0].validTo)
    }

    @Test
    fun `invalidate fact sets valid_to`() {
        val engine = HermesEngine.inMemory("test-inv")
        val store = TemporalStore(engine.db, "test-inv")
        val id = store.addFact(factType = FactType.DECISION, content = "Use SQLite for storage")
        store.invalidateFact(id)
        val active = store.getActiveFacts()
        assertTrue(active.isEmpty())
    }

    @Test
    fun `supersede fact creates chain`() {
        val engine = HermesEngine.inMemory("test-sup")
        val store = TemporalStore(engine.db, "test-sup")
        val oldId = store.addFact(factType = FactType.DECISION, content = "Use ChromaDB")
        val newId = store.addFact(factType = FactType.DECISION, content = "Use Qdrant instead")
        store.invalidateFact(oldId, supersededBy = newId)
        val active = store.getActiveFacts()
        assertEquals(1, active.size)
        assertEquals("Use Qdrant instead", active[0].content)
    }

    @Test
    fun `filter by fact type`() {
        val engine = HermesEngine.inMemory("test-filter")
        val store = TemporalStore(engine.db, "test-filter")
        store.addFact(factType = FactType.ARCHITECTURE, content = "Axum backend")
        store.addFact(factType = FactType.DECISION, content = "Use Rust")
        val archFacts = store.getActiveFacts(FactType.ARCHITECTURE)
        assertEquals(1, archFacts.size)
        assertEquals("Axum backend", archFacts[0].content)
    }

    @Test
    fun `fact type roundtrip`() {
        for (ft in FactType.entries) {
            assertEquals(ft, FactType.parse(ft.value))
        }
    }
}
