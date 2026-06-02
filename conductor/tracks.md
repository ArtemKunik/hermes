# Hermes Conductor Tracks

## Track Status Legend
| Status | Meaning |
|--------|---------|
| `completed` | All tasks done, verified, merged |
| `in-progress` | Currently being implemented |
| `planned` | Spec and plan approved, ready to start |
| `speccing` | Requirements being defined |
| `blocked` | Waiting on external dependency |
| `superseded` | Replaced by a newer track |
| `cancelled` | Dropped |

## Active Tracks

*None*

## Completed Tracks

### TRACK-067: Fix stale search results after file deletion
- **Description**: Fixed orphaned nodes surviving file deletion by removing `node_type='file'` filter from `get_all_file_paths()`
- **Outcome**: Single-line SQL fix in `src/graph_queries.rs:47`, regression tests added

### TRACK-068: Blast-Radius Scoring Engine
- **Description**: Per-node blast-radius scores computed during indexing via BFS limited to dependency edges. `blast_scores` table with weighted formula `direct + (0.5 × transitive)`, risk levels (HIGH >15%, MEDIUM >5%, LOW), and MCP tools `hermes_blast_score` + `hermes_high_blast`.
- **Outcome**: `src/blast_radius.rs` — 356 lines with BFS, scoring, get/query APIs. All tests pass.

### TRACK-069: Symbol Index + Fast Lookup
- **Description**: Dedicated `symbol_index` table for O(1) symbol-to-location lookup. Populated during ingestion from AST and regex chunkers. MCP tools `hermes_lookup` and `hermes_file_symbols`. Exported flag extraction, impl method tracking.
- **Outcome**: `src/symbol_index.rs` — 183 lines with insert/lookup/get_file_symbols/clear. All tests pass.

### TRACK-070: AGENTS.md Symbol Injection
- **Description**: CLI command `hermes inject-symbols [--path AGENTS.md] [--all] [--budget 2000]` writes compressed symbol tables between HTML comment markers, blast-score prioritized, token-budgeted, idempotent re-runs.
- **Outcome**: Wired in `src/bin/hermes/main.rs`, uses `src/symbol_inject.rs`. CLI flag parsing for --path, --all, --budget.

### TRACK-071: Pre-commit Hook
- **Description**: CLI command `hermes install-hook [--threshold 10] [--strict] [--remove]` generates `.git/hooks/pre-commit` shell script that queries blast_scores for staged files. Warns (or blocks with `--strict`) when files exceed threshold.
- **Outcome**: Wired in `src/bin/hermes/main.rs`, uses `src/hook.rs`. Supports --threshold, --strict, --remove flags.

### TRACK-072: Multi-language AST
- **Description**: Language registry (`src/ingestion/lang/`) with `LanguageExtractor` trait and per-language extractors for Rust, TypeScript/JSX, and Python. Refactored `ast_chunker.rs` and `xref_extractor.rs` to dispatch via registry. Feature-gated behind `ast` feature flag.
- **Outcome**: 4 new files in `src/ingestion/lang/`. All 66 ingestion tests pass (55 without ast, 66 with). Zero regression.

### TRACK-073: Embedded Visualization UI
- **Description**: `hermes serve --viz [--port 8080]` launches embedded HTTP server with 3 visualization modes: force-directed dependency graph (nodes colored by blast score), file-tree blast heatmap, and module treemap (area = LOC, color = blast score). Three JSON API endpoints: `/api/graph`, `/api/blast`, `/api/symbols/:file`.
- **Outcome**: 4 new files in `src/viz/`. Single HTML page with d3.js v7 from CDN, no build step required.

### TRACK-074: OpenCode Plugin Enhancement
- **Description**: Upgraded Hermes OpenCode TypeScript plugin from 17 to ~62 tools covering all Rust MCP tools. Added lifecycle hooks: blast-score write guard (warns on HIGH risk files after write/edit), enhanced session compaction (injects recall + top-5 blast + open findings), and AGENTS.md auto-injection on session start. Added `hermes_viz_graph` MCP tool returning d3-compatible `{nodes, edges}` JSON.
- **Outcome**: `tools/hermes-opencode-plugin/hermes.ts` — ~45 new tool wrappers, 3 lifecycle hooks (write guard, session compaction, auto-injection). `src/viz/api.rs` — `hermes_viz_graph` handler. All 293 lib tests pass. 16 warnings (pre-existing).
