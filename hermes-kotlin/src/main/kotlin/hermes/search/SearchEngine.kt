package hermes.search

import hermes.*
import java.io.File
import java.time.Duration
import java.time.Instant
import java.util.concurrent.ConcurrentHashMap

private const val CACHE_TTL_SECS = 60L
private const val CACHE_MAX_ENTRIES = 256
private const val FETCH_CACHE_MAX_ENTRIES = 50
private const val SHORT_CIRCUIT_SKIP_ALL = 0.9
private const val SHORT_CIRCUIT_SKIP_L2 = 0.8

enum class SearchMode {
    POINTER, SMART, FULL
}

data class SearchResult(
    val node: Node,
    val score: Double,
    val tier: SearchTier,
    val matchedContent: String? = null
)

enum class SearchTier {
    L0_LITERAL, L1_FTS, L2_VECTOR
}

class SearchEngine(
    private val graph: KnowledgeGraph,
    private val searchCache: SearchCacheMap
) {
    private val fetchCache = ConcurrentHashMap<Triple<String, Long, Long>, String>()

    fun search(query: String, topK: Int, mode: SearchMode): PointerResponse {
        val cacheKey = "${query.trim().lowercase()}:$topK"
        getFromCache(cacheKey)?.let { return it }

        val allResults = mutableListOf<SearchResult>()

        // L0: Literal
        val l0Results = literalSearch(graph, query)

        if (l0Results.size >= topK) {
            val minScore = l0Results.take(topK).minOfOrNull { it.score } ?: 0.0

            if (minScore >= SHORT_CIRCUIT_SKIP_ALL) {
                val merged = deduplicateAndRank(l0Results, topK)
                val pointers = resultsToPointers(merged)
                val response = PointerResponse.build(pointers, 0)
                insertIntoCache(cacheKey, response)
                return response
            }

            if (minScore >= SHORT_CIRCUIT_SKIP_L2) {
                allResults.addAll(l0Results)
                allResults.addAll(ftsSearch(graph, query))
                val merged = deduplicateAndRank(allResults, topK)
                val pointers = resultsToPointers(merged)
                val response = PointerResponse.build(pointers, 0)
                insertIntoCache(cacheKey, response)
                return response
            }
        }

        allResults.addAll(l0Results)

        // L1: FTS
        allResults.addAll(ftsSearch(graph, query))

        // L2: Vector
        allResults.addAll(vectorSearch(graph, query))

        val merged = deduplicateAndRank(allResults, topK)
        val pointers = resultsToPointers(merged)
        val response = PointerResponse.build(pointers, 0)
        insertIntoCache(cacheKey, response)
        return response
    }

    fun fetch(pointerId: String): FetchResponse? {
        val node = graph.getNode(pointerId) ?: return null

        val content = readNodeContentCached(node)
        val tokenCount = estimateTokens(content)

        return FetchResponse(
            pointerId = node.id,
            content = content,
            filePath = node.filePath ?: "",
            startLine = node.startLine ?: 0,
            endLine = node.endLine ?: 0,
            tokenCount = tokenCount
        )
    }

    private fun getFromCache(key: String): PointerResponse? {
        val ttl = Duration.ofSeconds(CACHE_TTL_SECS)
        val entry = searchCache[key] ?: return null
        if (Duration.between(entry.second, Instant.now()) < ttl) {
            return entry.first
        }
        searchCache.remove(key)
        return null
    }

    private fun insertIntoCache(key: String, response: PointerResponse) {
        if (searchCache.size >= CACHE_MAX_ENTRIES) {
            val ttl = Duration.ofSeconds(CACHE_TTL_SECS)
            val now = Instant.now()
            searchCache.entries.removeIf { Duration.between(it.value.second, now) >= ttl }
            if (searchCache.size >= CACHE_MAX_ENTRIES) {
                val oldest = searchCache.entries.minByOrNull { it.value.second }
                oldest?.let { searchCache.remove(it.key) }
            }
        }
        searchCache[key] = response to Instant.now()
    }

    private fun readNodeContentCached(node: Node): String {
        val filePath = node.filePath ?: ""
        val start = node.startLine ?: 0
        val end = node.endLine ?: 0
        val cacheKey = Triple(filePath, start, end)

        if (filePath.isNotEmpty()) {
            fetchCache[cacheKey]?.let { return it }
        }

        val content = readNodeContent(node)

        if (filePath.isNotEmpty()) {
            if (fetchCache.size >= FETCH_CACHE_MAX_ENTRIES) {
                fetchCache.keys.firstOrNull()?.let { fetchCache.remove(it) }
            }
            fetchCache[cacheKey] = content
        }
        return content
    }

    companion object {
        fun deduplicateAndRank(results: List<SearchResult>, topK: Int): List<SearchResult> {
            val best = mutableMapOf<String, SearchResult>()

            for (result in results) {
                val tierBonus = when (result.tier) {
                    SearchTier.L0_LITERAL -> 0.3
                    SearchTier.L1_FTS -> 0.1
                    SearchTier.L2_VECTOR -> 0.0
                }
                val boostedScore = result.score + tierBonus

                val existing = best[result.node.id]
                if (existing != null) {
                    val existingBonus = when (existing.tier) {
                        SearchTier.L0_LITERAL -> 0.3
                        SearchTier.L1_FTS -> 0.1
                        SearchTier.L2_VECTOR -> 0.0
                    }
                    if (boostedScore > existing.score + existingBonus) {
                        best[result.node.id] = result
                    }
                } else {
                    best[result.node.id] = result
                }
            }

            return best.values
                .sortedByDescending { it.score }
                .take(topK)
        }

        private fun resultsToPointers(results: List<SearchResult>): List<Pointer> {
            return results.map { r ->
                Pointer(
                    id = r.node.id,
                    source = r.node.filePath ?: "",
                    chunk = r.node.name,
                    lines = "${r.node.startLine ?: 0}-${r.node.endLine ?: 0}",
                    relevance = r.score,
                    summary = r.node.summary ?: "",
                    nodeType = r.node.nodeType.value,
                    lastModified = null
                )
            }
        }

        fun readNodeContent(node: Node): String {
            val path = node.filePath ?: return ""
            val file = File(path)

            val fileContent = try {
                file.readText()
            } catch (_: Exception) {
                return "[File not found: $path]"
            }

            val start = (node.startLine ?: 1).coerceAtLeast(1).toInt()
            val end = (node.endLine ?: 0).toInt()

            if (end == 0) return fileContent

            val lines = fileContent.lines()
            val startIdx = (start - 1).coerceAtMost(lines.size)
            val endIdx = end.coerceAtMost(lines.size)
            return lines.subList(startIdx, endIdx).joinToString("\n")
        }
    }
}

fun estimateTokens(content: String): Long {
    val wordCount = content.split("\\s+".toRegex()).count { it.isNotEmpty() }.toLong()
    return (wordCount * 4 + 2) / 3
}
