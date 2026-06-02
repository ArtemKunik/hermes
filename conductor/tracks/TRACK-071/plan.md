# Plan: TRACK-071 — Pre-commit Hook

## Phase 1 — Hook Generation

- [ ] Create `src/hook.rs` with `generate_hook_script()` function
- [ ] Implement shell script template with blast score threshold checking
- [ ] Script uses `sqlite3` CLI to query blast_scores (or cached JSON fallback)
- [ ] Format warning output: file path + blast score + risk level

## Phase 2 — Install/Remove Logic

- [ ] Implement `install_hook()`: write script to `.git/hooks/pre-commit`, chmod +x
- [ ] Implement `remove_hook()`: delete `.git/hooks/pre-commit` if hermes-managed
- [ ] Add hermes marker comment for safe removal detection

## Phase 3 — CLI Wiring + Tests

- [ ] Add `install-hook` subcommand to `src/bin/hermes/main.rs`
- [ ] Parse args: `--threshold`, `--strict`, `--remove`
- [ ] Unit test: hook script generation
- [ ] Unit test: install/remove round-trip with temp git repo
- [ ] Integration test: hook warns on high-blast file

## Files to Touch

| File | Change | Risk |
|------|--------|------|
| `src/hook.rs` | **NEW** — hook generation + install/remove | Low |
| `src/bin/hermes/main.rs` | Add CLI subcommand | Low |
| `src/lib.rs` | Declare hook module | Low |
