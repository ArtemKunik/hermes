package hermes.search

import hermes.KnowledgeGraph
import hermes.Node
import hermes.getAllNodes

private const val VECTOR_DIMENSION = 256
private const val VECTOR_LIMIT = 20
private const val MIN_SCORE = 0.20

fun vectorSearch(graph: KnowledgeGraph, query: String): List<SearchResult> {
    val queryTokens = tokenize(query)
    if (queryTokens.isEmpty()) return emptyList()

    val queryVec = buildVector(queryTokens)

    return graph.getAllNodes()
        .mapNotNull { node ->
            val text = combinedNodeText(node)
            val tokens = tokenize(text)
            if (tokens.isEmpty()) return@mapNotNull null

            val nodeVec = buildVector(tokens)
            val score = cosineSimilarity(queryVec, nodeVec)
            if (score < MIN_SCORE) return@mapNotNull null

            SearchResult(
                node = node,
                score = score,
                tier = SearchTier.L2_VECTOR,
                matchedContent = null
            )
        }
        .sortedByDescending { it.score }
        .take(VECTOR_LIMIT)
}

private fun combinedNodeText(node: Node): String {
    val sb = StringBuilder(node.name)
    node.summary?.let { sb.append(' ').append(it) }
    node.filePath?.let { sb.append(' ').append(it) }
    return sb.toString()
}

private fun tokenize(input: String): List<String> {
    return input.split(Regex("[^\\w]"))
        .map { it.trim().lowercase() }
        .filter { it.length > 1 }
}

private fun buildVector(tokens: List<String>): FloatArray {
    val vec = FloatArray(VECTOR_DIMENSION)
    for (token in tokens) {
        val index = stableHash(token) % VECTOR_DIMENSION
        vec[index] += 1.0f
    }
    normalize(vec)
    return vec
}

private fun stableHash(value: String): Int {
    // Use the same deterministic hash approach as Rust's DefaultHasher
    return value.hashCode().let { if (it < 0) -it else it }
}

private fun normalize(vec: FloatArray) {
    val norm = Math.sqrt(vec.sumOf { (it * it).toDouble() })
    if (norm < Double.MIN_VALUE) return
    for (i in vec.indices) {
        vec[i] = (vec[i] / norm).toFloat()
    }
}

private fun cosineSimilarity(lhs: FloatArray, rhs: FloatArray): Double {
    var sum = 0.0
    for (i in lhs.indices) {
        sum += lhs[i].toDouble() * rhs[i].toDouble()
    }
    return sum
}
