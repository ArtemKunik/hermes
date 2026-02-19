package hermes

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import java.sql.Connection
import java.time.ZonedDateTime
import java.time.format.DateTimeFormatter
import java.util.UUID

@Serializable
enum class FactType(val value: String) {
    @SerialName("architecture") ARCHITECTURE("architecture"),
    @SerialName("api_contract") API_CONTRACT("api_contract"),
    @SerialName("decision") DECISION("decision"),
    @SerialName("error_pattern") ERROR_PATTERN("error_pattern"),
    @SerialName("constraint") CONSTRAINT("constraint"),
    @SerialName("learning") LEARNING("learning");

    companion object {
        fun parse(s: String): FactType = entries.find { it.value == s } ?: DECISION
    }
}

@Serializable
data class TemporalFact(
    val id: String,
    val projectId: String,
    val nodeId: String? = null,
    val factType: FactType,
    val content: String,
    val validFrom: String,
    val validTo: String? = null,
    val supersededBy: String? = null,
    val sourceReference: String? = null
)

class TemporalStore(
    private val db: Connection,
    private val projectId: String
) {
    fun addFact(
        nodeId: String? = null,
        factType: FactType,
        content: String,
        sourceReference: String? = null
    ): String {
        val id = UUID.randomUUID().toString()
        val now = ZonedDateTime.now().format(DateTimeFormatter.ISO_OFFSET_DATE_TIME)

        db.prepareStatement(
            """INSERT INTO temporal_facts
               (id, project_id, node_id, fact_type, content, valid_from, source_reference)
               VALUES (?, ?, ?, ?, ?, ?, ?)"""
        ).use { ps ->
            ps.setString(1, id)
            ps.setString(2, projectId)
            ps.setString(3, nodeId)
            ps.setString(4, factType.value)
            ps.setString(5, content)
            ps.setString(6, now)
            ps.setString(7, sourceReference)
            ps.executeUpdate()
        }
        return id
    }

    fun invalidateFact(factId: String, supersededBy: String? = null) {
        val now = ZonedDateTime.now().format(DateTimeFormatter.ISO_OFFSET_DATE_TIME)
        db.prepareStatement(
            """UPDATE temporal_facts SET valid_to = ?, superseded_by = ?
               WHERE id = ? AND project_id = ?"""
        ).use { ps ->
            ps.setString(1, now)
            ps.setString(2, supersededBy)
            ps.setString(3, factId)
            ps.setString(4, projectId)
            ps.executeUpdate()
        }
    }

    fun getActiveFacts(factType: FactType? = null): List<TemporalFact> {
        val facts = mutableListOf<TemporalFact>()

        val (sql, params) = if (factType != null) {
            Pair(
                """SELECT id, project_id, node_id, fact_type, content, valid_from, valid_to, superseded_by, source_reference
                   FROM temporal_facts
                   WHERE project_id = ? AND valid_to IS NULL AND fact_type = ?
                   ORDER BY valid_from DESC""",
                listOf(projectId, factType.value)
            )
        } else {
            Pair(
                """SELECT id, project_id, node_id, fact_type, content, valid_from, valid_to, superseded_by, source_reference
                   FROM temporal_facts
                   WHERE project_id = ? AND valid_to IS NULL
                   ORDER BY valid_from DESC""",
                listOf(projectId)
            )
        }

        db.prepareStatement(sql).use { ps ->
            params.forEachIndexed { i, v -> ps.setString(i + 1, v) }
            ps.executeQuery().use { rs ->
                while (rs.next()) facts.add(mapRowToFact(rs))
            }
        }
        return facts
    }

    fun getFactHistory(nodeId: String): List<TemporalFact> {
        val facts = mutableListOf<TemporalFact>()
        db.prepareStatement(
            """SELECT id, project_id, node_id, fact_type, content, valid_from, valid_to, superseded_by, source_reference
               FROM temporal_facts
               WHERE project_id = ? AND node_id = ?
               ORDER BY valid_from DESC"""
        ).use { ps ->
            ps.setString(1, projectId)
            ps.setString(2, nodeId)
            ps.executeQuery().use { rs ->
                while (rs.next()) facts.add(mapRowToFact(rs))
            }
        }
        return facts
    }

    private fun mapRowToFact(rs: java.sql.ResultSet) = TemporalFact(
        id = rs.getString(1),
        projectId = rs.getString(2),
        nodeId = rs.getString(3),
        factType = FactType.parse(rs.getString(4)),
        content = rs.getString(5),
        validFrom = rs.getString(6),
        validTo = rs.getString(7),
        supersededBy = rs.getString(8),
        sourceReference = rs.getString(9)
    )
}
