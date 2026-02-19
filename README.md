# Hermes Engine

A Rust-native **knowledge graph engine** with pointer-based RAG (Retrieval-Augmented Generation), hybrid search, and temporal truth — designed for use as a Model Context Protocol (MCP) server in AI-assisted development workflows.

## Features

- **Pointer-based RAG**: Retrieve relevant code/doc nodes without pulling full content; fetch only when needed
- **Hybrid search**: Full-text search (FTS5), vector/embedding similarity, and literal pattern matching
- **Temporal fact store**: Record and query time-scoped facts (decisions, constraints, learnings, API contracts)
- **Knowledge graph**: File and symbol relationships built from workspace crawling and chunking
- **MCP server**: Exposes tools via the [Model Context Protocol](https://modelcontextprotocol.io/) for use with AI coding assistants (e.g. GitHub Copilot, Claude, Cursor)
- **Token accounting**: Tracks token savings from pointer-based retrieval vs. full file reads

## Architecture

```
src/
├── bin/hermes.rs       # MCP server entry point (stdio transport)
├── lib.rs              # Public API surface
├── schema.rs           # SQLite schema definitions
├── graph.rs            # Knowledge graph core
├── graph_builders.rs   # Graph construction helpers
├── graph_queries.rs    # Graph traversal queries
├── pointer.rs          # Pointer node types and resolution
├── accounting.rs       # Token savings accounting
├── embedding.rs        # Embedding generation (OpenAI-compatible)
├── temporal.rs         # Temporal fact store
├── mcp_server.rs       # MCP protocol implementation
├── ingestion/
│   ├── mod.rs          # Ingestion orchestration
│   ├── crawler.rs      # Workspace file crawler
│   ├── chunker.rs      # Code/text chunking
│   └── hash_tracker.rs # File change detection
└── search/
    ├── mod.rs          # Unified search interface
    ├── fts.rs          # Full-text search (SQLite FTS5)
    ├── vector.rs       # Vector similarity search
    └── literal.rs      # Literal/regex pattern search
```

## Quick Start

### Prerequisites

- Rust 1.75+
- An OpenAI-compatible embedding endpoint (or set `HERMES_EMBEDDINGS_DISABLED=1` to disable vector search)

### Build

```bash
cargo build --release
```

### Run as MCP Server

```bash
HERMES_PROJECT_ROOT=/path/to/your/project ./target/release/Hermes --stdio
```

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `HERMES_PROJECT_ROOT` | `.` | Root directory to index |
| `HERMES_DB_PATH` | `<project_root>/.hermes/hermes.db` | SQLite database path |
| `HERMES_EMBEDDINGS_URL` | `http://localhost:11434/v1/embeddings` | OpenAI-compatible embeddings endpoint |
| `HERMES_EMBEDDINGS_MODEL` | `nomic-embed-text` | Embedding model name |
| `HERMES_EMBEDDINGS_DISABLED` | `0` | Set to `1` to disable vector search |
| `HERMES_OPENAI_API_KEY` | *(unset)* | API key if using OpenAI embeddings |

## MCP Tools

| Tool | Description |
|------|-------------|
| `hermes_index` | Index/re-index the project files into the knowledge graph |
| `hermes_search` | Search the knowledge graph; returns pointers (not full content) |
| `hermes_fetch` | Fetch full content for a specific node by ID |
| `hermes_fact` | Record a persistent fact (decision, learning, constraint, etc.) |
| `hermes_facts` | List active facts, optionally filtered by type |
| `hermes_stats` | Return cumulative token savings statistics |

## VS Code Integration (MCP)

Add to `.vscode/mcp.json`:

```json
{
  "servers": {
    "hermes": {
      "type": "stdio",
      "command": "/path/to/hermes/target/release/Hermes",
      "args": ["--stdio"],
      "env": {
        "HERMES_PROJECT_ROOT": "${workspaceFolder}"
      }
    }
  }
}
```

## License

See [LICENSE](LICENSE).
