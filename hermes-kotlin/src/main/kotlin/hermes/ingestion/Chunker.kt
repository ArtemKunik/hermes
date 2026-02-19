package hermes.ingestion

import hermes.NodeType
import java.nio.file.Path

data class Chunk(
    val name: String,
    val nodeType: NodeType,
    val content: String,
    val startLine: Int,
    val endLine: Int,
    val summary: String
)

fun chunkFile(path: Path, content: String): List<Chunk> {
    val ext = path.toFile().extension
    return when (ext) {
        "rs" -> chunkRust(content)
        "kt", "kts", "java" -> chunkJvmLike(content)
        "md" -> chunkMarkdown(content)
        "tsx", "ts", "jsx", "js" -> chunkTypescript(content)
        else -> chunkWholeFile(path, content)
    }
}

// ── Rust chunker ──────────────────────────────────────────────────────────

private fun chunkRust(content: String): List<Chunk> {
    val chunks = mutableListOf<Chunk>()
    val lines = content.lines()
    var i = 0
    while (i < lines.size) {
        val line = lines[i].trim()
        val chunk = tryParseRustItem(line, lines, i)
        if (chunk != null) chunks.add(chunk)
        i++
    }
    return chunks
}

private fun tryParseRustItem(line: String, lines: List<String>, start: Int): Chunk? {
    val (name, nodeType) = when {
        line.startsWith("pub fn ") || line.startsWith("fn ") ||
        line.startsWith("pub async fn ") || line.startsWith("async fn ") ->
            (extractFnName(line) ?: return null) to NodeType.FUNCTION

        line.startsWith("pub struct ") || line.startsWith("struct ") ->
            (extractAfterKeyword(line, "struct") ?: return null) to NodeType.STRUCT

        line.startsWith("pub enum ") || line.startsWith("enum ") ->
            (extractAfterKeyword(line, "enum") ?: return null) to NodeType.ENUM

        line.startsWith("impl ") ->
            (extractImplName(line) ?: return null) to NodeType.IMPL

        line.startsWith("pub trait ") || line.startsWith("trait ") ->
            (extractAfterKeyword(line, "trait") ?: return null) to NodeType.TRAIT

        else -> return null
    }

    val end = findBlockEnd(lines, start)
    val blockContent = lines.subList(start, end + 1).joinToString("\n")
    val summary = buildSummary(name, nodeType, lines[start])

    return Chunk(
        name = name,
        nodeType = nodeType,
        content = blockContent,
        startLine = start + 1,
        endLine = end + 1,
        summary = summary
    )
}

// ── JVM-like (Kotlin/Java) chunker ───────────────────────────────────────

private fun chunkJvmLike(content: String): List<Chunk> {
    val chunks = mutableListOf<Chunk>()
    val lines = content.lines()

    for (i in lines.indices) {
        val trimmed = lines[i].trim()
        val (name, nodeType) = when {
            trimmed.contains("fun ") && trimmed.contains("(") ->
                (extractJvmFnName(trimmed) ?: continue) to NodeType.FUNCTION

            (trimmed.startsWith("class ") || trimmed.startsWith("data class ") ||
             trimmed.startsWith("open class ") || trimmed.startsWith("abstract class ") ||
             trimmed.contains(" class ")) && trimmed.contains("{") ->
                (extractAfterKeyword(trimmed, "class") ?: continue) to NodeType.STRUCT

            trimmed.startsWith("interface ") || trimmed.contains(" interface ") ->
                (extractAfterKeyword(trimmed, "interface") ?: continue) to NodeType.TRAIT

            trimmed.startsWith("enum class ") || trimmed.contains(" enum class ") ->
                (extractAfterKeyword(trimmed, "class") ?: continue) to NodeType.ENUM

            trimmed.startsWith("object ") || trimmed.contains(" object ") ->
                (extractAfterKeyword(trimmed, "object") ?: continue) to NodeType.MODULE

            else -> continue
        }

        val end = findBlockEnd(lines, i)
        val blockContent = lines.subList(i, end + 1).joinToString("\n")
        val summary = buildSummary(name, nodeType, lines[i])

        chunks.add(Chunk(
            name = name,
            nodeType = nodeType,
            content = blockContent,
            startLine = i + 1,
            endLine = end + 1,
            summary = summary
        ))
    }
    return chunks
}

private fun extractJvmFnName(line: String): String? {
    val afterFun = line.split("fun ").getOrNull(1) ?: return null
    val name = afterFun.split("(").firstOrNull()?.split("<")?.firstOrNull()?.trim()
    return if (name.isNullOrEmpty()) null else name
}

// ── Markdown chunker ─────────────────────────────────────────────────────

private fun chunkMarkdown(content: String): List<Chunk> {
    val chunks = mutableListOf<Chunk>()
    val lines = content.lines()
    var sectionStart: Pair<Int, String>? = null

    for (i in lines.indices) {
        val line = lines[i]
        if (line.startsWith("## ") || line.startsWith("# ")) {
            sectionStart?.let { (start, heading) ->
                chunks.add(Chunk(
                    name = heading,
                    nodeType = NodeType.DOCUMENT,
                    content = lines.subList(start, i).joinToString("\n"),
                    startLine = start + 1,
                    endLine = i,
                    summary = heading
                ))
            }
            sectionStart = i to line.trimStart('#').trim()
        }
    }
    sectionStart?.let { (start, heading) ->
        chunks.add(Chunk(
            name = heading,
            nodeType = NodeType.DOCUMENT,
            content = lines.subList(start, lines.size).joinToString("\n"),
            startLine = start + 1,
            endLine = lines.size,
            summary = heading
        ))
    }
    return chunks
}

// ── TypeScript/JS chunker ────────────────────────────────────────────────

private fun chunkTypescript(content: String): List<Chunk> {
    val chunks = mutableListOf<Chunk>()
    val lines = content.lines()

    for (i in lines.indices) {
        val trimmed = lines[i].trim()
        if (isTsFunctionStart(trimmed) || isTsComponentStart(trimmed)) {
            val name = extractTsName(trimmed) ?: "anonymous_$i"
            val end = findBlockEnd(lines, i)
            chunks.add(Chunk(
                name = name,
                nodeType = NodeType.FUNCTION,
                content = lines.subList(i, end + 1).joinToString("\n"),
                startLine = i + 1,
                endLine = end + 1,
                summary = "TypeScript function: $name"
            ))
        }
    }
    return chunks
}

// ── Whole-file fallback ──────────────────────────────────────────────────

private fun chunkWholeFile(path: Path, content: String): List<Chunk> {
    val name = path.fileName.toString()
    return listOf(Chunk(
        name = name,
        nodeType = NodeType.FILE,
        content = content,
        startLine = 1,
        endLine = content.lines().size,
        summary = "File: $name"
    ))
}

// ── Helpers ──────────────────────────────────────────────────────────────

private fun extractFnName(line: String): String? {
    val afterFn = line.split("fn ").getOrNull(1) ?: return null
    val name = afterFn.split("(").firstOrNull()?.split("<")?.firstOrNull()?.trim()
    return if (name.isNullOrEmpty()) null else name
}

private fun extractAfterKeyword(line: String, keyword: String): String? {
    val after = line.split("$keyword ").getOrNull(1) ?: return null
    val name = after.split("{").firstOrNull()
        ?.split("<")?.firstOrNull()
        ?.split("(")?.firstOrNull()
        ?.split(":")?.firstOrNull()
        ?.trim()
    return if (name.isNullOrEmpty()) null else name
}

private fun extractImplName(line: String): String? {
    val afterImpl = line.removePrefix("impl ").ifEmpty { return null }
    val name = afterImpl.split("{").firstOrNull()
        ?.split("for ")?.lastOrNull()
        ?.split("<")?.firstOrNull()
        ?.trim()
    return if (name.isNullOrEmpty()) null else name
}

private fun findBlockEnd(lines: List<String>, start: Int): Int {
    var depth = 0
    var foundOpen = false

    for (i in start until lines.size) {
        for (ch in lines[i]) {
            if (ch == '{') { depth++; foundOpen = true }
            else if (ch == '}') depth--
        }
        if (foundOpen && depth <= 0) return i
    }
    return (start + 1).coerceAtMost(lines.size - 1)
}

private fun buildSummary(name: String, nodeType: NodeType, firstLine: String): String {
    val cleanLine = firstLine.trim()
    return if (cleanLine.length > 80) "${nodeType.value}: $name"
    else "${nodeType.value}: $cleanLine"
}

private fun isTsFunctionStart(line: String): Boolean {
    return (line.startsWith("export function ") ||
            line.startsWith("function ") ||
            line.startsWith("export const ") ||
            line.startsWith("const ")) &&
            (line.contains("=>") || line.contains("("))
}

private fun isTsComponentStart(line: String): Boolean {
    return line.startsWith("export default function ") || line.startsWith("export default class ")
}

private fun extractTsName(line: String): String? {
    for (keyword in listOf("function ", "const ", "class ")) {
        val after = line.split(keyword).getOrNull(1) ?: continue
        val name = after.split("(").firstOrNull()
            ?.split("=")?.firstOrNull()
            ?.split(":")?.firstOrNull()
            ?.split("<")?.firstOrNull()
            ?.trim()
        if (!name.isNullOrEmpty()) return name
    }
    return null
}
