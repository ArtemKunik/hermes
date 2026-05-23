# TRACK-067: Fix stale search results after file deletion

## Plan

### Task 1 — Write regression test (fail first) [ ]

**Goal**: Lock down the bug pattern with a deterministic failing test.

**Steps**:
1. Create nodes for `foo.rs`: one `File` node + one `Function` node, both with `file_path = "foo.rs"`
2. Delete only the `File` node via `graph.delete_node()` (simulates partial cleanup)
3. Run a search query that would match `foo.rs` content
4. Assert: the Function node's path still appears in results → **test fails** (bug confirmed)

**Seam**: `src/graph_queries.rs` — test uses `HermesEngine::in_memory()` + direct graph ops, no network needed.

### Task 2 — Fix `get_all_file_paths` to return ALL paths [ ]

**Goal**: Make `cleanup_stale_nodes()` see every path stored in the database.

**Change**: `src/graph_queries.rs:43-52`
```sql
-- Before (bug):
WHERE project_id = ?1 AND node_type = 'file' AND file_path IS NOT NULL

-- After (fix):
WHERE project_id = ?1 AND file_path IS NOT NULL
```

**Why this works**: Every indexed node type (File, Function, Class, Method, etc.) stores its source `file_path`. By removing the `node_type='file'` filter, cleanup sees all orphaned paths.

**Risk**: Low. This is a single-line SQL change. The query already has `project_id` scoping.

### Task 3 — Verify regression test passes [ ]

1. Run the test from Task 1 → should pass
2. Run `cargo test` → no regressions
3. Manual verification: delete a file, re-index, search for it → should not appear

### Task 4 (optional) — Invalidate cache after background ingestion [ ]

**Goal**: Fix stale in-memory search results after `remember`/`write_decision`.

**Change**: Add `engine.invalidate_search_cache()` call after background ingest threads complete.

**Files affected**:
- `src/mcp_memory/session.rs:85-92` (tool_remember)
- `src/mcp_memory/decision.rs:76-83` (tool_write_decision)

**Risk**: Medium. Background threads don't have direct access to the engine instance — requires passing a cache invalidation handle or using a channel.

### Task 5 (optional) — Memory indexer cleanup [ ]

**Goal**: Remove Qdrant points for deleted `.md` files during memory index runs.

**Change**: `src/memory_indexer.rs:run()` — track which file paths were found, delete Qdrant points for any previously-indexed path not in the current crawl set.

**Risk**: Medium-High. Requires tracking historical paths and making delete calls to Qdrant API. Needs careful handling of point IDs (deterministic from file_path + section_heading).

---

## Execution Order

1. **Tasks 1-3 are a vertical slice** — they fix the primary bug with minimal risk
2. **Task 4 is a quick win** if the cache invalidation pattern is straightforward
3. **Task 5 should be a separate track** (TRACK-068) since it touches Qdrant integration

## Files to Touch

| File | Change | Risk |
|------|--------|------|
| `src/graph_queries.rs:47` | Remove `node_type = 'file'` filter from query | Low |
| `src/ingestion/mod.rs:182-189` | No change needed (cleanup logic already correct) | — |
| `src/mcp_memory/session.rs:85-92` | Add cache invalidation after background ingest (Task 4) | Medium |
| `src/mcp_memory/decision.rs:76-83` | Same as above (Task 4) | Medium |

## Verification

1. **Unit test** (Task 1): deterministic fail→pass signal
2. **Integration**: delete a file → re-index → search for deleted filename → should return no results
3. **Regression**: run `cargo test` — all existing tests pass
