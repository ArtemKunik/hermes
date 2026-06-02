# Plan: TRACK-073 — Embedded Visualization UI

## Phase 1 — HTTP Server

- [ ] Add `tiny_http` dependency to `Cargo.toml`
- [ ] Create `src/viz/mod.rs` module root
- [ ] Create `src/viz/server.rs` with HTTP server + routing
- [ ] Implement static file serving for embedded HTML
- [ ] Implement graceful shutdown on SIGINT

## Phase 2 — API Endpoints

- [ ] Create `src/viz/api.rs` with JSON API handlers
- [ ] Implement `GET /api/graph` — nodes + edges from graph + blast_scores
- [ ] Implement `GET /api/blast` — blast scores with optional threshold filter
- [ ] Implement `GET /api/symbols/:file` — symbols from symbol_index

## Phase 3 — Frontend

- [ ] Create `src/viz/static/index.html` — single-page app
- [ ] Implement dependency graph visualization (d3 force layout)
- [ ] Implement blast heatmap (d3 tree layout)
- [ ] Implement module treemap (d3 treemap layout)
- [ ] Implement tab switching between modes
- [ ] Implement click-to-detail side panel
- [ ] Embed CSS for dark theme

## Phase 4 — CLI Wiring + Tests

- [ ] Add `serve --viz` subcommand to `src/bin/hermes/main.rs`
- [ ] Parse args: `--port`, `--repo`
- [ ] Unit test: API endpoint JSON output format
- [ ] Integration test: server starts, serves HTML, responds to API calls
- [ ] Manual test: open in browser, verify all 3 viz modes

## Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| Large graph crashes browser | Medium | Limit to top-N nodes by blast score, cluster directories |
| CDN unavailable (offline) | Low | Fallback message, or bundle d3.min.js as embedded asset |
| Port conflicts | Low | Configurable port, clear error message on bind failure |

## Files to Touch

| File | Change | Risk |
|------|--------|------|
| `Cargo.toml` | Add tiny_http dependency | Low |
| `src/viz/mod.rs` | **NEW** — module root | Low |
| `src/viz/server.rs` | **NEW** — HTTP server | Medium |
| `src/viz/api.rs` | **NEW** — API handlers | Low |
| `src/viz/static/index.html` | **NEW** — visualization UI | Medium |
| `src/bin/hermes/main.rs` | Add serve --viz subcommand | Low |
| `src/lib.rs` | Declare viz module | Low |
