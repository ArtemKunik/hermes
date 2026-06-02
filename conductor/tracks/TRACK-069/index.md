# TRACK-069: Symbol Index + Fast Lookup

**Status**: completed
**Created**: 2026-05-29
**Type**: Feature / Search
**Depends on**: None — can start immediately
**Branch**: `feat/symbol-index`

## Goal

Create a dedicated `symbol_index` table for O(1) symbol-to-location lookup, exposed via lightweight MCP tools. Eliminate the need for full 3-tier search when agents just need to find where a function or struct is defined.

## Problem

Finding a symbol's location currently requires `hermes_search` which runs literal → FTS → vector search. For the common case of "where is `verify_token` defined?", this is overkill. The `name_to_id` map built during ingestion already does this but is ephemeral and not queryable.

## What to Build

- `symbol_index` table: `name → file_path:line + kind + exported + methods`
- Populated during ingestion from both AST and regex chunkers
- `hermes_lookup` MCP tool: O(1) name → location(s)
- `hermes_file_symbols` MCP tool: all symbols in a file

## Acceptance Criteria

- [ ] `symbol_index` table created with migration
- [ ] Symbols inserted during `ingest_file()` for both AST and regex paths
- [ ] `exported` flag extracted (pub keyword / export keyword)
- [ ] Impl block methods extracted as comma-separated list
- [ ] `hermes_lookup` returns all matches for a symbol name
- [ ] `hermes_file_symbols` returns all symbols in a file
- [ ] Stale symbols cleared on re-index (matches existing node cleanup)
- [ ] All existing tests pass, new tests added
- [ ] Tool count test updated to 62

## Related Tracks

- TRACK-070: AGENTS.md Symbol Injection (reads symbol_index)
- TRACK-072: Multi-language AST (populates symbol_index for TS/Python)
- TRACK-073: Visualization UI (symbol detail panels)

## Links

- [Specification](./spec.md)
- [Implementation Plan](./plan.md)
