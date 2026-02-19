package hermes

import kotlinx.serialization.Serializable
import java.sql.Connection
import java.time.Duration

@Serializable
data class CumulativeStats(
    val totalQueries: Long,
    val totalPointerTokens: Long,
    val totalFetchedTokens: Long,
    val totalTraditionalEstimate: Long,
    val cumulativeSavingsTokens: Long,
    val cumulativeSavingsPct: Double
)

class Accountant(
    private val db: Connection,
    private val projectId: String,
    private val sessionId: String
) {
    fun recordQuery(
        queryText: String,
        pointerTokens: Long,
        fetchedTokens: Long,
        traditionalEstimate: Long
    ) {
        db.prepareStatement(
            """INSERT INTO accounting (project_id, session_id, query_text, pointer_tokens, fetched_tokens, traditional_est)
               VALUES (?, ?, ?, ?, ?, ?)"""
        ).use { ps ->
            ps.setString(1, projectId)
            ps.setString(2, sessionId)
            ps.setString(3, queryText)
            ps.setLong(4, pointerTokens)
            ps.setLong(5, fetchedTokens)
            ps.setLong(6, traditionalEstimate)
            ps.executeUpdate()
        }
    }

    fun getCumulativeStats(): CumulativeStats = getStatsSince(null)

    fun getStatsSince(since: Duration?): CumulativeStats {
        val (query, params) = if (since != null) {
            val secs = since.seconds
            Pair(
                """SELECT COUNT(*),
                          COALESCE(SUM(pointer_tokens), 0),
                          COALESCE(SUM(fetched_tokens), 0),
                          COALESCE(SUM(traditional_est), 0)
                   FROM accounting
                   WHERE project_id = ?
                     AND created_at >= datetime('now', '-$secs seconds')""",
                listOf(projectId)
            )
        } else {
            Pair(
                """SELECT COUNT(*),
                          COALESCE(SUM(pointer_tokens), 0),
                          COALESCE(SUM(fetched_tokens), 0),
                          COALESCE(SUM(traditional_est), 0)
                   FROM accounting WHERE project_id = ?""",
                listOf(projectId)
            )
        }

        return db.prepareStatement(query).use { ps ->
            params.forEachIndexed { i, v -> ps.setString(i + 1, v) }
            ps.executeQuery().use { rs ->
                rs.next()
                buildStats(rs)
            }
        }
    }

    fun getSessionStats(): CumulativeStats {
        return db.prepareStatement(
            """SELECT COUNT(*),
                      COALESCE(SUM(pointer_tokens), 0),
                      COALESCE(SUM(fetched_tokens), 0),
                      COALESCE(SUM(traditional_est), 0)
               FROM accounting WHERE project_id = ? AND session_id = ?"""
        ).use { ps ->
            ps.setString(1, projectId)
            ps.setString(2, sessionId)
            ps.executeQuery().use { rs ->
                rs.next()
                buildStats(rs)
            }
        }
    }

    private fun buildStats(rs: java.sql.ResultSet): CumulativeStats {
        val totalQueries = rs.getLong(1)
        val ptrTokens = rs.getLong(2)
        val fetchTokens = rs.getLong(3)
        val tradEst = rs.getLong(4)
        val actual = ptrTokens + fetchTokens
        val saved = (tradEst - actual).coerceAtLeast(0)
        val pct = if (tradEst > 0) (saved.toDouble() / tradEst.toDouble()) * 100.0 else 0.0
        return CumulativeStats(
            totalQueries = totalQueries,
            totalPointerTokens = ptrTokens,
            totalFetchedTokens = fetchTokens,
            totalTraditionalEstimate = tradEst,
            cumulativeSavingsTokens = saved,
            cumulativeSavingsPct = pct
        )
    }
}

fun parseSinceDuration(s: String): Duration? {
    val trimmed = s.trim().lowercase()
    return when {
        trimmed == "all" -> null
        trimmed.endsWith("h") -> {
            val hours = trimmed.removeSuffix("h").toLongOrNull() ?: return null
            Duration.ofHours(hours)
        }
        trimmed.endsWith("d") -> {
            val days = trimmed.removeSuffix("d").toLongOrNull() ?: return null
            Duration.ofDays(days)
        }
        else -> null
    }
}
