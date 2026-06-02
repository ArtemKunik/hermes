# Plan: TRACK-069 — Symbol Index + Fast Lookup

## Phase 1 — Schema + CRUD

- [ ] Add `symbol_index` table creation to `src/schema_tables.rs`
- [ ] Create `src/symbol_index.rs` with `SymbolEntry` struct
- [ ] Implement `insert_symbol()`, `lookup_symbol()`, `get_file_symbols()`, `clear_file_symbols()`
- [ ] Declare module in `src/lib.rs`

## Phase 2 — Ingestion Integration

- [ ] In `src/ingestion/file_ops.rs`: insert into `symbol_index` after each symbol node creation (AST path)
- [ ] In `src/ingestion/file_ops.rs`: insert into `symbol_index` after each regex chunk (regex path)
- [ ] Extract `exported` flag: check for `pub` in Rust, `export` in TS/JS
- [ ] Extract methods from impl blocks: parse `fn` names within `impl` content
- [ ] Add `clear_file_symbols()` call in cleanup path (before `delete_nodes_for_file()`)

## Phase 3 — MCP Tools

- [ ] Implement `tool_lookup()` in new or existing tools file
- [ ] Implement `tool_file_symbols()` in new or existing tools file
- [ ] Add schemas to `src/mcp_tool_schemas/core.rs`
- [ ] Register handlers in `src/mcp_actor_dispatch.rs`
- [ ] Add both tools to `READ_ONLY_TOOLS` set
- [ ] Add `hermes_lookup` to CORE profile in `src/tool_router.rs`
- [ ] Add `hermes_file_symbols` to STANDARD profile

## Phase 4 — Tests

- [ ] Unit test: insert + lookup round-trip
- [ ] Unit test: lookup returns multiple matches (same name, different files)
- [ ] Unit test: clear_file_symbols removes entries
- [ ] Unit test: exported flag extraction
- [ ] Integration test: symbols populated after ingestion
- [ ] Update `registry_has_expected_count` test to 62

## Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| Name collisions (many `new`, `default`, `run`) | Medium | Return all matches, let caller disambiguate |
| Impl method extraction fragile | Low | Start with regex, improve with AST when available |
| Table grows with re-indexes | Low | UNIQUE constraint + clear-before-reingest pattern |

## Files to Touch

| File | Change | Risk |
|------|--------|------|
| `src/schema_tables.rs` | Add symbol_index table | Low |
| `src/symbol_index.rs` | **NEW** — CRUD operations | Low |
| `src/lib.rs` | Declare symbol_index module | Low |
| `src/ingestion/file_ops.rs` | Insert symbols during ingestion | Medium |
| `src/mcp_actor_dispatch.rs` | Register 2 handlers | Low |
| `src/mcp_tool_schemas/core.rs` | 2 new schemas | Low |
| `src/tool_router.rs` | Add to CORE/STANDARD profiles | Low |
