package hermes.ingestion

import hermes.EdgeType
import hermes.KnowledgeGraph
import hermes.NodeType
import hermes.deleteNodesForFile
import hermes.getAllFilePaths
import java.nio.charset.Charset
import java.nio.file.Path

data class IngestionReport(
    val totalFiles: Int = 0,
    var indexed: Int = 0,
    var skipped: Int = 0,
    var errors: Int = 0,
    var nodesCreated: Int = 0
) {
    override fun toString(): String =
        "Ingestion: $totalFiles files ($indexed indexed, $skipped skipped, $errors errors), $nodesCreated nodes"
}

class IngestionPipeline(
    private val graph: KnowledgeGraph
) {
    private val hashTracker = HashTracker(graph.db, graph.projectId)

    fun ingestDirectory(dirPath: Path): IngestionReport {
        val files = crawlDirectory(dirPath)

        val crawledPaths = files.map { it.toString() }.toSet()

        val report = IngestionReport(totalFiles = files.size)

        val toIngest = mutableListOf<Path>()
        for (filePath in files) {
            val pathStr = filePath.toString()
            if (hashTracker.isUnchanged(pathStr)) {
                report.skipped++
            } else {
                toIngest.add(filePath)
            }
        }

        // Process files (sequential — Kotlin coroutines could parallelize later)
        for (filePath in toIngest) {
            val pathStr = filePath.toString()
            try {
                val count = ingestFile(filePath)
                report.indexed++
                report.nodesCreated += count
                hashTracker.updateHash(pathStr, filePath)
            } catch (e: Exception) {
                System.err.println("[hermes] Failed to ingest $pathStr: ${e.message}")
                report.errors++
            }
        }

        cleanupStaleNodes(crawledPaths)

        return report
    }

    private fun cleanupStaleNodes(crawledPaths: Set<String>) {
        val dbPaths = graph.getAllFilePaths()
        for (stalePath in dbPaths - crawledPaths) {
            graph.deleteNodesForFile(stalePath)
            System.err.println("[hermes] Removed stale nodes for deleted file: $stalePath")
        }
    }

    fun ingestFile(filePath: Path): Int {
        // Read as bytes and convert lossily to handle non-UTF-8 files
        val bytes = filePath.toFile().readBytes()
        val content = String(bytes, Charset.forName("UTF-8"))
        val pathStr = filePath.toString()
        val chunks = chunkFile(filePath, content)

        val fileHash = computeHash(content)
        val fileNode = graph.createNodeBuilder()
            .name(pathStr)
            .nodeType(NodeType.FILE)
            .filePath(pathStr)
            .lines(1L, content.lines().size.toLong())
            .contentHash(fileHash)
            .build()

        graph.addNode(fileNode)
        graph.indexFts(fileNode, content)

        var created = 1

        for (chunk in chunks) {
            val chunkKey = "$pathStr::${chunk.name}"
            val chunkHash = computeHash(chunk.content)

            if (hashTracker.isChunkUnchanged(chunkKey, chunkHash)) continue

            val chunkNode = graph.createNodeBuilder()
                .name(chunk.name)
                .nodeType(chunk.nodeType)
                .filePath(pathStr)
                .lines(chunk.startLine.toLong(), chunk.endLine.toLong())
                .summary(chunk.summary)
                .build()

            graph.addNode(chunkNode)
            graph.indexFts(chunkNode, chunk.content)

            val edge = graph.createEdgeBuilder()
                .source(fileNode.id)
                .target(chunkNode.id)
                .edgeType(EdgeType.CONTAINS)
                .build()

            graph.addEdge(edge)
            hashTracker.updateChunkHash(chunkKey, chunkHash)
            created++
        }

        return created
    }
}
