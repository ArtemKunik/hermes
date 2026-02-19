package hermes

import java.nio.file.Path
import java.sql.Connection
import java.sql.DriverManager
import java.time.Instant
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap

/**
 * Thread-safe wrapper around a SQLite connection.
 */
typealias SearchCacheMap = ConcurrentHashMap<String, Pair<PointerResponse, Instant>>

class HermesEngine private constructor(
    val db: Connection,
    val projectId: String,
    val sessionId: String = UUID.randomUUID().toString(),
    val searchCache: SearchCacheMap = SearchCacheMap()
) {
    companion object {
        /**
         * Open (or create) a database at the given file path.
         */
        fun open(dbPath: Path, projectId: String): HermesEngine {
            Class.forName("org.sqlite.JDBC")
            val conn = DriverManager.getConnection("jdbc:sqlite:${dbPath.toAbsolutePath()}")
            conn.createStatement().use { stmt ->
                stmt.execute("PRAGMA journal_mode=WAL")
                stmt.execute("PRAGMA synchronous=NORMAL")
            }
            Schema.runMigrations(conn)
            return HermesEngine(db = conn, projectId = projectId)
        }

        /**
         * Create an in-memory engine (useful for testing).
         */
        fun inMemory(projectId: String): HermesEngine {
            Class.forName("org.sqlite.JDBC")
            val conn = DriverManager.getConnection("jdbc:sqlite::memory:")
            Schema.runMigrations(conn)
            return HermesEngine(db = conn, projectId = projectId)
        }
    }

    /**
     * Clear the search cache.
     */
    fun invalidateSearchCache() {
        searchCache.clear()
    }
}
