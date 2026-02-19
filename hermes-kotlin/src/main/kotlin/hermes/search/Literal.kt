package hermes.search

import hermes.KnowledgeGraph
import hermes.literalSearchByName

fun literalSearch(graph: KnowledgeGraph, query: String): List<SearchResult> {
    val queryLower = query.lowercase()
    val nodes = graph.literalSearchByName(query)

    val results = nodes.map { node ->
        val nameLower = node.name.lowercase()
        SearchResult(
            node = node,
            score = computeLiteralScore(queryLower, nameLower),
            tier = SearchTier.L0_LITERAL,
            matchedContent = null
        )
    }
        .sortedByDescending { it.score }
        .take(20)

    return results
}

private fun computeLiteralScore(query: String, name: String): Double {
    if (name == query) return 1.0
    if (name.startsWith(query) || name.endsWith(query)) return 0.9
    val queryLen = query.length.toDouble()
    val nameLen = name.length.coerceAtLeast(1).toDouble()
    return 0.5 + (queryLen / nameLen) * 0.4
}
