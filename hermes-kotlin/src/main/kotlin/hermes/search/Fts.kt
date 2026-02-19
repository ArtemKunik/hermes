package hermes.search

import hermes.KnowledgeGraph
import hermes.ftsSearch

private const val FTS_LIMIT = 20
private const val STRATEGY_MIN_RESULTS = 3
private const val MAX_QUERY_WORDS = 10

/**
 * Returns true for characters that belong to scripts without whitespace word
 * boundaries (CJK ideographs, Hiragana, Katakana, Hangul).
 */
private fun isCjk(ch: Char): Boolean {
    return ch in '\u3040'..'\u309F' ||   // Hiragana
           ch in '\u30A0'..'\u30FF' ||   // Katakana
           ch in '\u3400'..'\u4DBF' ||   // CJK Extension A
           ch in '\u4E00'..'\u9FFF' ||   // CJK Unified Ideographs
           ch in '\uF900'..'\uFAFF' ||   // CJK Compatibility
           ch in '\uAC00'..'\uD7AF'      // Hangul Syllables
}

/**
 * Extracts alphanumeric/underscore tokens from the raw query and removes
 * FTS operators. CJK characters are emitted individually.
 */
fun extractWords(query: String): List<String> {
    val words = mutableListOf<String>()
    val cur = StringBuilder()

    for (ch in query) {
        if (isCjk(ch)) {
            if (cur.isNotEmpty()) {
                if (!isFtsOperator(cur.toString())) words.add(cur.toString())
                cur.clear()
            }
            words.add(ch.toString())
        } else if (ch.isLetterOrDigit() || ch == '_') {
            cur.append(ch)
        } else if (cur.isNotEmpty()) {
            if (!isFtsOperator(cur.toString())) words.add(cur.toString())
            cur.clear()
        }
    }
    if (cur.isNotEmpty() && !isFtsOperator(cur.toString())) {
        words.add(cur.toString())
    }
    return words.take(MAX_QUERY_WORDS)
}

fun ftsSearch(graph: KnowledgeGraph, query: String): List<SearchResult> {
    val words = extractWords(query)
    if (words.isEmpty()) return emptyList()

    if (words.size == 1) {
        val single = "\"${words[0]}\""
        return toSearchResults(graph.ftsSearch(single, FTS_LIMIT))
    }

    // Strategy 1: phrase match
    val phraseQuery = "\"${words.joinToString(" ")}\""
    val s1 = graph.ftsSearch(phraseQuery, FTS_LIMIT)
    if (s1.size >= STRATEGY_MIN_RESULTS) return toSearchResults(s1)

    // Strategy 2: AND with prefix
    val andQuery = words.joinToString(" AND ") { "\"$it\"*" }
    val s2 = graph.ftsSearch(andQuery, FTS_LIMIT)
    if (s2.size >= STRATEGY_MIN_RESULTS) return toSearchResults(s2)

    // Strategy 3: OR
    val orQuery = words.joinToString(" OR ") { "\"$it\"" }
    return toSearchResults(graph.ftsSearch(orQuery, FTS_LIMIT))
}

private fun toSearchResults(raw: List<Pair<hermes.Node, Double>>): List<SearchResult> {
    return raw.map { (node, rank) ->
        SearchResult(
            node = node,
            score = normalizeBm25Score(rank),
            tier = SearchTier.L1_FTS,
            matchedContent = null
        )
    }
}

private fun isFtsOperator(word: String): Boolean {
    return word.uppercase() in listOf("AND", "OR", "NOT", "NEAR")
}

fun normalizeBm25Score(rank: Double): Double {
    val absRank = kotlin.math.abs(rank)
    if (absRank < 0.001) return 0.5
    return (1.0 - 1.0 / (1.0 + absRank)).coerceAtMost(1.0)
}
