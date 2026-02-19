package hermes

import hermes.ingestion.IngestionPipeline
import hermes.search.SearchEngine
import hermes.search.SearchMode
import hermes.search.estimateTokens
import kotlinx.serialization.json.*
import java.nio.file.Path
import java.nio.file.Paths

private val json = Json { prettyPrint = true; ignoreUnknownKeys = true }

fun main(args: Array<String>) {
    if (args.isEmpty()) {
        printUsage()
        return
    }

    val (engine, projectRoot) = openEngine()
    val command = args[0]

    if (command == "--stdio") {
        McpServer.run(engine, projectRoot)
        return
    }

    when (command) {
        "index" -> cmdIndex(engine, projectRoot)
        "search" -> {
            val query = args.getOrNull(1) ?: run { System.err.println("usage: hermes search <query>"); return }
            cmdSearch(engine, query)
        }
        "fetch" -> {
            val id = args.getOrNull(1) ?: run { System.err.println("usage: hermes fetch <node_id>"); return }
            cmdFetch(engine, id)
        }
        "fact" -> {
            val factType = args.getOrNull(1) ?: ""
            val content = args.getOrNull(2) ?: ""
            if (factType.isEmpty() || content.isEmpty()) {
                System.err.println("usage: hermes fact <type> <content>")
                return
            }
            cmdAddFact(engine, factType, content)
        }
        "facts" -> {
            val filter = args.getOrNull(1)
            cmdListFacts(engine, filter)
        }
        "stats" -> {
            val sinceArg = args.getOrNull(1)
            cmdStats(engine, sinceArg)
        }
        else -> {
            printUsage()
            System.err.println("unknown command: $command")
        }
    }
}

private fun openEngine(): Pair<HermesEngine, Path> {
    val projectRoot = System.getenv("HERMES_PROJECT_ROOT")
        ?.let { Paths.get(it) }
        ?: Paths.get(System.getProperty("user.dir", "."))

    val dbPath = System.getenv("HERMES_DB_PATH")
        ?.let { Paths.get(it) }
        ?: projectRoot.resolve(".hermes.db")

    val projectId = projectRoot.fileName?.toString() ?: "unknown"

    val engine = HermesEngine.open(dbPath, projectId)
    return engine to projectRoot
}

private fun cmdIndex(engine: HermesEngine, projectRoot: Path) {
    val graph = KnowledgeGraph(engine.db, engine.projectId)
    val pipeline = IngestionPipeline(graph)
    val report = pipeline.ingestDirectory(projectRoot)
    engine.invalidateSearchCache()
    val output = buildJsonObject {
        put("total_files", report.totalFiles)
        put("indexed", report.indexed)
        put("skipped", report.skipped)
        put("errors", report.errors)
        put("nodes_created", report.nodesCreated)
    }
    println(json.encodeToString(JsonElement.serializer(), output))
}

private fun cmdSearch(engine: HermesEngine, query: String) {
    val graph = KnowledgeGraph(engine.db, engine.projectId)
    val search = SearchEngine(graph, engine.searchCache)
    val response = search.search(query, 10, SearchMode.SMART)

    val acct = Accountant(engine.db, engine.projectId, engine.sessionId)
    acct.recordQuery(query, response.accounting.pointerTokens, 0, response.accounting.traditionalRagEstimate)

    println(json.encodeToString(PointerResponse.serializer(), response))
}

private fun cmdFetch(engine: HermesEngine, nodeId: String) {
    val graph = KnowledgeGraph(engine.db, engine.projectId)
    val search = SearchEngine(graph, engine.searchCache)
    val response = search.fetch(nodeId) ?: run {
        System.err.println("node not found: $nodeId")
        return
    }

    val traditionalEstimate = response.tokenCount * 15
    val acct = Accountant(engine.db, engine.projectId, engine.sessionId)
    acct.recordQuery(nodeId, 0, response.tokenCount, traditionalEstimate)

    println(json.encodeToString(FetchResponse.serializer(), response))
}

private fun cmdAddFact(engine: HermesEngine, factTypeStr: String, content: String) {
    val store = TemporalStore(engine.db, engine.projectId)
    val factType = FactType.parse(factTypeStr)
    val id = store.addFact(factType = factType, content = content)
    println(json.encodeToString(JsonElement.serializer(), buildJsonObject {
        put("id", id); put("status", "recorded")
    }))
}

private fun cmdListFacts(engine: HermesEngine, filter: String?) {
    val store = TemporalStore(engine.db, engine.projectId)
    val factType = filter?.let { FactType.parse(it) }
    val facts = store.getActiveFacts(factType)
    println(json.encodeToString(
        kotlinx.serialization.builtins.ListSerializer(TemporalFact.serializer()),
        facts
    ))
}

private fun cmdStats(engine: HermesEngine, sinceArg: String?) {
    val acct = Accountant(engine.db, engine.projectId, engine.sessionId)
    val session = acct.getSessionStats()

    val sinceDur = sinceArg?.let { parseSinceDuration(it) }
    val cumulative = acct.getStatsSince(sinceDur)

    val sinceLabel = sinceArg ?: "all"
    val output = buildJsonObject {
        put("project_id", engine.projectId)
        put("since_filter", sinceLabel)
        putJsonObject("session") {
            put("total_queries", session.totalQueries)
            put("pointer_tokens_used", session.totalPointerTokens)
            put("fetched_tokens_used", session.totalFetchedTokens)
            put("actual_tokens_total", session.totalPointerTokens + session.totalFetchedTokens)
            put("traditional_rag_estimate", session.totalTraditionalEstimate)
            put("tokens_saved", session.cumulativeSavingsTokens)
            put("savings_pct", "%.1f%%".format(session.cumulativeSavingsPct))
        }
        putJsonObject("cumulative") {
            put("total_queries", cumulative.totalQueries)
            put("pointer_tokens_used", cumulative.totalPointerTokens)
            put("fetched_tokens_used", cumulative.totalFetchedTokens)
            put("actual_tokens_total", cumulative.totalPointerTokens + cumulative.totalFetchedTokens)
            put("traditional_rag_estimate", cumulative.totalTraditionalEstimate)
            put("tokens_saved", cumulative.cumulativeSavingsTokens)
            put("savings_pct", "%.1f%%".format(cumulative.cumulativeSavingsPct))
        }
    }
    println(json.encodeToString(JsonElement.serializer(), output))
}

private fun printUsage() {
    System.err.println(
        """
        hermes — token-efficient code navigation (Kotlin edition)

        USAGE: hermes <command> [args]

        Commands:
          index               Re-index the project (run when files change)
          search <query>      Search codebase; returns pointers (no full content)
          fetch <node_id>     Fetch full content for a specific pointer
          fact <type> <text>  Record a decision/learning (types: architecture, decision,
                              learning, constraint, error_pattern, api_contract)
          facts [type]        List active facts, optionally filtered by type
          stats [duration]    Show token savings (duration: 24h, 7d, 30d, all)
          --stdio             Run as MCP JSON-RPC 2.0 stdio server (for VS Code Copilot)

        Env vars:
          HERMES_PROJECT_ROOT             Root directory to index (default: .)
          HERMES_DB_PATH                  SQLite database path (default: <root>/.hermes.db)
          HERMES_AUTO_INDEX_INTERVAL_SECS Auto-reindex interval (default: 300, 0=off)
          GEMINI_API_KEY                  API key for Gemini embeddings
          GEMINI_EMBEDDING_MODEL          Embedding model (default: text-embedding-004)
        """.trimIndent()
    )
}
