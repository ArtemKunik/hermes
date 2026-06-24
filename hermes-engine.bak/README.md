# Hermes

Hermes is the SmartPositionAssistant knowledge engine. It indexes a project into a SQLite-backed knowledge graph and exposes CLI and MCP tools for search, recall, architecture checks, symbol validation, and session memory.

## Build

```powershell
cargo build --release
```

## Test

```powershell
cargo test
```

## Use With SmartPositionAssistant

SmartPositionAssistant consumes this repository as `tools/hermes-engine`. Keep `HERMES_PROJECT_ROOT` pointed at the workspace that should be indexed, not necessarily this repository checkout.