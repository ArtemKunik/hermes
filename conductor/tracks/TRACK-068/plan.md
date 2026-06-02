# Plan: TRACK-068 — Blast-Radius Scoring Engine

## Phase 1 — Schema + Migration

- [ ] Add `blast_scores` table creation to `src/schema_tables.rs`
- [ ] Add `idx_edges_project_type` index to `src/schema_tables.rs`
- [ ] Verify migration runs cleanly on existing DB

## Phase 2 — Computation Module

- [ ] Create `src/blast_radius.rs` with `BlastScore` struct and `RiskLevel` enum
- [ ] Implement `build_adjacency_list()` — single query to load all dependency edges
- [ ] Implement `compute_blast_scores()` — batch BFS over adjacency list
- [ ] Implement `upsert_blast_scores()` — batch INSERT OR REPLACE
- [ ] Declare module in `src/lib.rs`

## Phase 3 — Indexing Integration

- [ ] Call `compute_all_blast_scores()` at end of `IngestionPipeline::ingest_directory()` after xref phase
- [ ] Ensure scores are computed inside the same transaction
- [ ] Add logging for computation time and score count

## Phase 4 — MCP Tools

- [ ] Implement `tool_blast_score()` in `src/mcp_tools_graph.rs`
- [ ] Implement `tool_high_blast()` in `src/mcp_tools_graph.rs`
- [ ] Add schemas to `src/mcp_tool_schemas/arch.rs`
- [ ] Register handlers in `src/mcp_actor_dispatch.rs`
- [ ] Add both tools to `READ_ONLY_TOOLS` set
- [ ] Add both tools to STANDARD profile in `src/tool_router.rs`
- [ ] Enhance `tool_impact_analysis()`: filter BFS to dependency edges, add risk_level/direct/transitive/percentage to output

## Phase 5 — Tests

- [ ] Unit test: blast score formula with known graph topology
- [ ] Unit test: risk level thresholds (HIGH/MEDIUM/LOW)
- [ ] Unit test: edge type filtering (Contains/Documents excluded)
- [ ] Integration test: scores computed during ingestion
- [ ] Update `registry_has_expected_count` test to 60

## Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| Large graph BFS slow on big repos | Medium | In-memory adjacency list, batch computation |
| Edge type filter excludes useful edges | Low | Start conservative (5 dependency types), expand if needed |
| Score persistence grows table unbounded | Low | Cleanup stale scores when nodes are deleted |

## Files to Touch

| File | Change | Risk |
|------|--------|------|
| `src/schema_tables.rs` | Add blast_scores table + edge index | Low |
| `src/blast_radius.rs` | **NEW** — computation module | Low |
| `src/lib.rs` | Declare blast_radius module | Low |
| `src/ingestion/mod.rs` | Call compute after xref phase | Medium |
| `src/mcp_tools_graph.rs` | 2 new tools + enhance impact_analysis | Medium |
| `src/mcp_actor_dispatch.rs` | Register 2 handlers + READ_ONLY | Low |
| `src/mcp_tool_schemas/arch.rs` | 2 new schemas | Low |
| `src/tool_router.rs` | Add to STANDARD profile | Low |
