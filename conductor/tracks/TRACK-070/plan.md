# Plan: TRACK-070 — AGENTS.md Symbol Injection

## Phase 1 — Core Logic

- [ ] Create `src/symbol_inject.rs` with `inject_symbols()` function
- [ ] Implement compressed format: `file: name:kind:line[methods...]`
- [ ] Implement kind abbreviation mapping (fn, st, en, tr, im, md, if, co)
- [ ] Implement token counting for budget enforcement
- [ ] Query symbol_index JOIN blast_scores ORDER BY blast_score DESC

## Phase 2 — File I/O

- [ ] Implement marker detection: find `<!-- hermes-symbols-start -->` / `<!-- hermes-symbols-end -->`
- [ ] Implement upsert: insert markers if missing, replace content if present
- [ ] Handle file creation if AGENTS.md doesn't exist

## Phase 3 — CLI Wiring + Tests

- [ ] Add `inject-symbols` subcommand to `src/bin/hermes/main.rs`
- [ ] Parse args: `--path`, `--all`, `--budget`
- [ ] Unit test: format output for known symbols
- [ ] Unit test: token budget truncation
- [ ] Unit test: idempotent re-run
- [ ] Integration test: full inject into temp AGENTS.md

## Files to Touch

| File | Change | Risk |
|------|--------|------|
| `src/symbol_inject.rs` | **NEW** — injection logic | Low |
| `src/bin/hermes/main.rs` | Add CLI subcommand | Low |
| `src/lib.rs` | Declare symbol_inject module | Low |
