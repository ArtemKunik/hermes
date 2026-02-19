# Hermes Engine (Kotlin)

A Kotlin/JVM port of the **Hermes knowledge graph engine** with pointer-based RAG, hybrid search, and temporal truth — designed as a Model Context Protocol (MCP) server for AI-assisted development workflows.

## Features

Same feature set as the Rust original:

- **Pointer-based RAG**: Retrieve relevant code/doc nodes without pulling full content
- **Hybrid search**: FTS5, vector similarity, and literal pattern matching
- **Temporal fact store**: Record and query time-scoped facts
- **Knowledge graph**: File and symbol relationships via workspace crawling/chunking
- **MCP server**: JSON-RPC 2.0 stdio server for VS Code Copilot, Claude, Cursor, etc.
- **Token accounting**: Tracks savings from pointer-based retrieval

## Prerequisites

- JDK 17+
- Gradle 8.5+ (wrapper included)

## Build

```bash
cd hermes-kotlin
./gradlew build
```

## Run as MCP Server

```bash
HERMES_PROJECT_ROOT=/path/to/project java -jar build/libs/hermes-engine-0.1.0.jar --stdio
```

Or directly via Gradle:
```bash
HERMES_PROJECT_ROOT=/path/to/project ./gradlew run --args="--stdio"
```

## CLI Usage

```bash
# Index your project
./gradlew run --args="index"

# Search
./gradlew run --args="search 'main function'"

# Fetch full node content
./gradlew run --args="fetch <node_id>"

# Record a fact
./gradlew run --args="fact decision 'Use Kotlin for the backend'"

# List active facts
./gradlew run --args="facts"

# Stats
./gradlew run --args="stats"
```

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `HERMES_PROJECT_ROOT` | `.` | Root directory to index |
| `HERMES_DB_PATH` | `<root>/.hermes.db` | SQLite database path |
| `HERMES_AUTO_INDEX_INTERVAL_SECS` | `300` | Auto-reindex interval (0 = disabled) |
| `GEMINI_API_KEY` | *(unset)* | API key for Gemini embeddings |
| `GEMINI_EMBEDDING_MODEL` | `text-embedding-004` | Embedding model name |

## MCP Tools

| Tool | Description |
|------|-------------|
| `hermes_index` | Index/re-index project files |
| `hermes_search` | Search knowledge graph (returns pointers) |
| `hermes_fetch` | Fetch full content by node ID |
| `hermes_fact` | Record a persistent fact |
| `hermes_facts` | List active facts |
| `hermes_stats` | Token savings statistics |

## VS Code Integration (MCP)

Add to `.vscode/mcp.json`:

```json
{
  "servers": {
    "hermes": {
      "type": "stdio",
      "command": "java",
      "args": ["-jar", "/path/to/hermes-kotlin/build/libs/hermes-engine-0.1.0.jar", "--stdio"],
      "env": {
        "HERMES_PROJECT_ROOT": "${workspaceFolder}"
      }
    }
  }
}
```

## Project Structure

```
src/main/kotlin/hermes/
├── Main.kt              # CLI entry point
├── HermesEngine.kt      # Public API surface
├── Schema.kt            # SQLite schema
├── Graph.kt             # Knowledge graph core
├── GraphBuilders.kt     # Node/Edge builders
├── GraphQueries.kt      # Graph traversal queries
├── Pointer.kt           # Pointer types and accounting
├── Accounting.kt        # Token savings tracking
├── Embedding.kt         # Gemini embedding client
├── Temporal.kt          # Temporal fact store
├── McpServer.kt         # MCP protocol (JSON-RPC 2.0)
├── ingestion/
│   ├── IngestionPipeline.kt   # Orchestration
│   ├── Crawler.kt             # File crawler
│   ├── Chunker.kt             # Code/text chunking
│   └── HashTracker.kt         # Change detection
└── search/
    ├── SearchEngine.kt        # Unified search
    ├── Fts.kt                 # Full-text search
    ├── Literal.kt             # Literal name matching
    └── Vector.kt              # Vector similarity
```

## Running Tests

```bash
./gradlew test
```
