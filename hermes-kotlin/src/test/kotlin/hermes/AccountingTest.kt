package hermes

import org.junit.jupiter.api.Test
import org.junit.jupiter.api.Assertions.*
import java.time.Duration

class AccountingTest {

    @Test
    fun `record and aggregate queries`() {
        val engine = HermesEngine.inMemory("test-acct")
        val acct = Accountant(engine.db, "test-acct", engine.sessionId)

        acct.recordQuery("find main function", 300, 0, 15000)
        acct.recordQuery("search currency service", 250, 1200, 12000)

        val stats = acct.getCumulativeStats()
        assertEquals(2, stats.totalQueries)
        assertEquals(550, stats.totalPointerTokens)
        assertEquals(1200, stats.totalFetchedTokens)
        assertEquals(27000, stats.totalTraditionalEstimate)
        assertEquals(25250, stats.cumulativeSavingsTokens)
        assertTrue(stats.cumulativeSavingsPct > 90.0)
    }

    @Test
    fun `empty stats returns zeros`() {
        val engine = HermesEngine.inMemory("test-empty")
        val acct = Accountant(engine.db, "test-empty", engine.sessionId)
        val stats = acct.getCumulativeStats()
        assertEquals(0, stats.totalQueries)
        assertEquals(0.0, stats.cumulativeSavingsPct)
    }

    @Test
    fun `parse since 24h`() {
        val dur = parseSinceDuration("24h")!!
        assertEquals(86400L, dur.seconds)
    }

    @Test
    fun `parse since 7d`() {
        val dur = parseSinceDuration("7d")!!
        assertEquals(7 * 86400L, dur.seconds)
    }

    @Test
    fun `parse since all returns null`() {
        assertNull(parseSinceDuration("all"))
    }

    @Test
    fun `parse since invalid returns null`() {
        assertNull(parseSinceDuration("yesterday"))
        assertNull(parseSinceDuration(""))
        assertNull(parseSinceDuration("abc"))
    }
}
