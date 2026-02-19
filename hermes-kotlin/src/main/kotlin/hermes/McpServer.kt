package hermes

import hermes.ingestion.IngestionPipeline
import hermes.search.SearchEngine
import hermes.search.SearchMode
import kotlinx.serialization.json.*
import java.io.BufferedReader
import java.io.InputStreamReader
import java.io.PrintWriter
import java.nio.file.Path
import kotlin.concurrent.thread

private val json = Json { prettyPrint = true; ignoreUnknownKeys = true }
private val jsonCompact = Json { ignoreUnknownKeys = true }

object McpServer {

    fun run(engine: HermesEngine, projectRoot: Path) {
        spawnAutoReindex(engine, projectRoot)

        val reader = BufferedReader(InputStreamReader(System.`in`))
        val writer = PrintWriter(System.out, true)

        while (true) {
            val line = reader.readLine() ?: break
            if (line.isBlank()) continue

            val msg = try {
                Json.parseToJsonElement(line).jsonObject
            } catch (e: Exception) {
                writeError(writer, JsonNull, -32700, "parse error: ${e.message}")
                continue
            }

            val id = msg["id"] ?: JsonNull
            val method = msg["method"]?.jsonPrimitive?.contentOrNull ?: ""
            val params = msg["params"] ?: JsonNull

            if (method.startsWith("notifications/")) continue

            try {
                val result = dispatch(engine, projectRoot, method, params)
                writeOk(writer, id, result)
            } catch (e: Exception) {
                writeError(writer, id, -32603, e.message ?: "internal error")
            }
        }
    }

    private fun spawnAutoReindex(engine: HermesEngine, projectRoot: Path) {
        val intervalSecs = System.getenv("HERMES_AUTO_INDEX_INTERVAL_SECS")
            ?.toLongOrNull() ?: 300L

        if (intervalSecs == 0L) {
            System.err.println("[hermes] auto-reindex disabled (HERMES_AUTO_INDEX_INTERVAL_SECS=0)")
            return
        }

        thread(isDaemon = true, name = "hermes-auto-reindex") {
            System.err.println("[hermes] auto-reindex thread started (interval=${intervalSecs}s)")
            while (true) {
                Thread.sleep(intervalSecs * 1000)
                try {
                    val graph = KnowledgeGraph(engine.db, engine.projectId)
                    val pipeline = IngestionPipeline(graph)
                    val report = pipeline.ingestDirectory(projectRoot)
                    System.err.println(
                        "[hermes] auto-reindex complete: ${report.indexed} indexed, ${report.skipped} skipped, ${report.errors} errors"
                    )
                } catch (e: Exception) {
                    System.err.println("[hermes] auto-reindex failed: ${e.message}")
                }
            }
        }
    }

    private fun dispatch(engine: HermesEngine, projectRoot: Path, method: String, params: JsonElement): JsonElement {
        return when (method) {
            "initialize" -> handleInitialize()
            "tools/list" -> handleToolsList()
            "tools/call" -> handleToolCall(engine, projectRoot, params)
            else -> throw RuntimeException("unknown method: $method")
        }
    }

    private fun handleInitialize(): JsonElement = buildJsonObject {
        put("protocolVersion", "2024-11-05")
        putJsonObject("capabilities") {
            putJsonObject("tools") { put("listChanged", false) }
        }
        putJsonObject("serverInfo") {
            put("name", "Hermes")
            put("version", "0.1.0")
        }
    }

    private fun handleToolsList(): JsonElement = buildJsonObject {
        putJsonArray("tools") {
            addJsonObject {
                put("name", "hermes_search")
                put("description", "Search the codebase knowledge graph. Returns pointers (not full content). Records token savings in accounting.")
                putJsonObject("inputSchema") {
                    put("type", "object")
                    putJsonObject("properties") {
                        putJsonObject("query") { put("type", "string"); put("description", "Natural-language or keyword search query") }
                    }
                    putJsonArray("required") { add("query") }
                }
            }
            addJsonObject {
                put("name", "hermes_fetch")
                put("description", "Fetch full content for a specific knowledge-graph node by ID returned by hermes_search.")
                putJsonObject("inputSchema") {
                    put("type", "object")
                    putJsonObject("properties") {
                        putJsonObject("node_id") { put("type", "string"); put("description", "Node ID from a previous search result") }
                    }
                    putJsonArray("required") { add("node_id") }
                }
            }
            addJsonObject {
                put("name", "hermes_index")
                put("description", "Re-index the project files into the knowledge graph. Run after adding or changing files.")
                putJsonObject("inputSchema") { put("type", "object"); putJsonObject("properties") {} }
            }
            addJsonObject {
                put("name", "hermes_stats")
                put("description", "Return cumulative token savings statistics across all Hermes sessions.")
                putJsonObject("inputSchema") { put("type", "object"); putJsonObject("properties") {} }
            }
            addJsonObject {
                put("name", "hermes_fact")
                put("description", "Record a persistent fact (decision, learning, constraint, etc.) into the temporal store.")
                putJsonObject("inputSchema") {
                    put("type", "object")
                    putJsonObject("properties") {
                        putJsonObject("fact_type") { put("type", "string"); put("description", "One of: architecture, decision, learning, constraint, error_pattern, api_contract") }
                        putJsonObject("content") { put("type", "string"); put("description", "The fact to record") }
                    }
                    putJsonArray("required") { add("fact_type"); add("content") }
                }
            }
            addJsonObject {
                put("name", "hermes_facts")
                put("description", "List active facts from the temporal store, optionally filtered by type.")
                putJsonObject("inputSchema") {
                    put("type", "object")
                    putJsonObject("properties") {
                        putJsonObject("fact_type") { put("type", "string"); put("description", "Optional filter type (omit for all)") }
                    }
                }
            }
        }
    }

    private fun handleToolCall(engine: HermesEngine, projectRoot: Path, params: JsonElement): JsonElement {
        val obj = params.jsonObject
        val name = obj["name"]?.jsonPrimitive?.contentOrNull ?: ""
        val args = obj["arguments"] ?: JsonNull

        val text = when (name) {
            "hermes_search" -> {
                val query = args.jsonObject["query"]?.jsonPrimitive?.contentOrNull ?: ""
                require(query.isNotEmpty()) { "hermes_search requires 'query'" }
                toolSearch(engine, query)
            }
            "hermes_fetch" -> {
                val nodeId = args.jsonObject["node_id"]?.jsonPrimitive?.contentOrNull ?: ""
                require(nodeId.isNotEmpty()) { "hermes_fetch requires 'node_id'" }
                toolFetch(engine, nodeId)
            }
            "hermes_index" -> toolIndex(engine, projectRoot)
            "hermes_stats" -> toolStats(engine)
            "hermes_fact" -> {
                val ft = args.jsonObject["fact_type"]?.jsonPrimitive?.contentOrNull ?: ""
                val c = args.jsonObject["content"]?.jsonPrimitive?.contentOrNull ?: ""
                require(ft.isNotEmpty() && c.isNotEmpty()) { "hermes_fact requires 'fact_type' and 'content'" }
                toolAddFact(engine, ft, c)
            }
            "hermes_facts" -> {
                val filter = args.jsonObject["fact_type"]?.jsonPrimitive?.contentOrNull
                toolListFacts(engine, filter)
            }
            else -> throw RuntimeException("unknown tool: $name")
        }

        return buildJsonObject {
            putJsonArray("content") {
                addJsonObject { put("type", "text"); put("text", text) }
            }
        }
    }

    private fun toolSearch(engine: HermesEngine, query: String): String {
        val graph = KnowledgeGraph(engine.db, engine.projectId)
        val search = SearchEngine(graph, engine.searchCache)
        val resp = search.search(query, 10, SearchMode.SMART)
        val acct = Accountant(engine.db, engine.projectId, engine.sessionId)
        acct.recordQuery(query, resp.accounting.pointerTokens, 0, resp.accounting.traditionalRagEstimate)
        return json.encodeToString(PointerResponse.serializer(), resp)
    }

    private fun toolFetch(engine: HermesEngine, nodeId: String): String {
        val graph = KnowledgeGraph(engine.db, engine.projectId)
        val search = SearchEngine(graph, engine.searchCache)
        val resp = search.fetch(nodeId) ?: throw RuntimeException("node not found: $nodeId")
        val acct = Accountant(engine.db, engine.projectId, engine.sessionId)
        acct.recordQuery(nodeId, 0, resp.tokenCount, resp.tokenCount * 15)
        return json.encodeToString(FetchResponse.serializer(), resp)
    }

    private fun toolIndex(engine: HermesEngine, projectRoot: Path): String {
        val graph = KnowledgeGraph(engine.db, engine.projectId)
        val pipeline = IngestionPipeline(graph)
        val report = pipeline.ingestDirectory(projectRoot)
        engine.invalidateSearchCache()
        return json.encodeToString(JsonElement.serializer(), buildJsonObject {
            put("total_files", report.totalFiles)
            put("indexed", report.indexed)
            put("skipped", report.skipped)
            put("errors", report.errors)
            put("nodes_created", report.nodesCreated)
        })
    }

    private fun toolStats(engine: HermesEngine): String {
        val acct = Accountant(engine.db, engine.projectId, engine.sessionId)
        val session = acct.getSessionStats()
        val cumulative = acct.getCumulativeStats()
        return json.encodeToString(JsonElement.serializer(), buildJsonObject {
            putJsonObject("session") {
                put("total_queries", session.totalQueries)
                put("pointer_tokens_used", session.totalPointerTokens)
                put("fetched_tokens_used", session.totalFetchedTokens)
                put("traditional_rag_estimate", session.totalTraditionalEstimate)
                put("tokens_saved", session.cumulativeSavingsTokens)
                put("savings_pct", "%.1f%%".format(session.cumulativeSavingsPct))
            }
            putJsonObject("cumulative") {
                put("total_queries", cumulative.totalQueries)
                put("pointer_tokens_used", cumulative.totalPointerTokens)
                put("fetched_tokens_used", cumulative.totalFetchedTokens)
                put("traditional_rag_estimate", cumulative.totalTraditionalEstimate)
                put("tokens_saved", cumulative.cumulativeSavingsTokens)
                put("savings_pct", "%.1f%%".format(cumulative.cumulativeSavingsPct))
            }
        })
    }

    private fun toolAddFact(engine: HermesEngine, factTypeStr: String, content: String): String {
        val store = TemporalStore(engine.db, engine.projectId)
        val id = store.addFact(factType = FactType.parse(factTypeStr), content = content)
        return json.encodeToString(JsonElement.serializer(), buildJsonObject {
            put("id", id); put("status", "recorded")
        })
    }

    private fun toolListFacts(engine: HermesEngine, filter: String?): String {
        val store = TemporalStore(engine.db, engine.projectId)
        val facts = store.getActiveFacts(filter?.let { FactType.parse(it) })
        return json.encodeToString(kotlinx.serialization.builtins.ListSerializer(TemporalFact.serializer()), facts)
    }

    private fun writeOk(writer: PrintWriter, id: JsonElement, result: JsonElement) {
        val envelope = buildJsonObject {
            put("jsonrpc", "2.0")
            put("id", id)
            put("result", result)
        }
        writer.println(jsonCompact.encodeToString(JsonElement.serializer(), envelope))
        writer.flush()
    }

    private fun writeError(writer: PrintWriter, id: JsonElement, code: Int, message: String) {
        val envelope = buildJsonObject {
            put("jsonrpc", "2.0")
            put("id", id)
            putJsonObject("error") { put("code", code); put("message", message) }
        }
        writer.println(jsonCompact.encodeToString(JsonElement.serializer(), envelope))
        writer.flush()
    }
}
