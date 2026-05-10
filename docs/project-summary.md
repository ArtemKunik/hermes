# Hermes Project Summary

## Design

Hermes is a Rust-native knowledge engine built for agent-assisted development. The design centers on pointer-based retrieval instead of dumping full file contents, so callers get compact references first and fetch details only when needed.

The system is optimized for local, repeatable use:

- SQLite is the primary storage layer.
- Search is layered so exact matches, keyword search, and semantic search can coexist.
- Temporal facts preserve decisions, learnings, and constraints over time.
- MCP exposes the engine to other tools and agents without requiring custom integrations.

## Functions

Hermes provides:

- Workspace indexing and re-indexing
- Hybrid search over code, docs, and symbols
- Pointer-based fetch of relevant nodes
- Persistent fact tracking for decisions and learnings
- Token accounting to measure savings from pointer-based retrieval
- Environment variable validation and consistency checks
- Automatic reindexing in the background
- MCP tools for agent workflows

In practice, Hermes acts as a navigation and memory layer for large codebases.

## Architecture

Hermes is organized as a single Rust workspace with a main engine crate and a small companion workspace member:

- `src/bin/hermes.rs` is the CLI and MCP entry point.
- `src/ingestion/` crawls files, chunks content, and tracks changes.
- `src/search/` unifies literal, FTS5, and vector search.
- `src/graph.rs` and related modules store the knowledge graph.
- `src/temporal.rs` stores facts with time context.
- `src/mcp_server.rs` and `src/mcp_tools_validation.rs` expose MCP behavior.
- `hermes-mind/` holds the companion workspace member.

The overall flow is:

1. Crawl and chunk the workspace.
2. Build graph nodes and edges in SQLite.
3. Answer queries with pointers and relevance metadata.
4. Fetch full content only when a caller explicitly needs it.
5. Record stats and facts so the engine improves over time.

