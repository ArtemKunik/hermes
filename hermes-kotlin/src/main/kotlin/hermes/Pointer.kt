package hermes

import kotlinx.serialization.Serializable

@Serializable
data class Pointer(
    val id: String,
    val source: String,
    val chunk: String,
    val lines: String,
    val relevance: Double,
    val summary: String,
    val nodeType: String,
    val lastModified: String? = null
) {
    fun estimateTokenCount(): Long {
        val text = "$source $chunk $lines $summary"
        val wordCount = text.split("\\s+".toRegex()).count { it.isNotEmpty() }.toLong()
        return (wordCount * 4 + 2) / 3 + 2
    }
}

@Serializable
data class AccountingReport(
    val pointerTokens: Long,
    val fetchedTokens: Long,
    val totalTokens: Long,
    val traditionalRagEstimate: Long,
    val savingsPct: Double
)

@Serializable
data class PointerResponse(
    val pointers: List<Pointer>,
    val accounting: AccountingReport
) {
    companion object {
        fun build(pointers: List<Pointer>, fetchedTokens: Long): PointerResponse {
            val pointerTokens = pointers.sumOf { it.estimateTokenCount() }
            val traditionalEstimate = pointerTokens * 15
            val total = pointerTokens + fetchedTokens
            val savingsPct = if (traditionalEstimate > 0) {
                (1.0 - total.toDouble() / traditionalEstimate.toDouble()) * 100.0
            } else {
                0.0
            }

            return PointerResponse(
                pointers = pointers,
                accounting = AccountingReport(
                    pointerTokens = pointerTokens,
                    fetchedTokens = fetchedTokens,
                    totalTokens = total,
                    traditionalRagEstimate = traditionalEstimate,
                    savingsPct = savingsPct.coerceAtLeast(0.0)
                )
            )
        }
    }
}

@Serializable
data class FetchResponse(
    val pointerId: String,
    val content: String,
    val filePath: String,
    val startLine: Long,
    val endLine: Long,
    val tokenCount: Long
)
