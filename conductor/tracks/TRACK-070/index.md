# TRACK-070: AGENTS.md Symbol Injection

**Status**: completed
**Created**: 2026-05-29
**Type**: Feature / Developer Experience
**Depends on**: TRACK-068 (blast scores for priority), TRACK-069 (symbol_index for data)
**Branch**: `feat/symbol-injection`

## Goal

Provide a CLI command that writes a compressed symbol table into AGENTS.md so AI agents can find any symbol in one glance without calling tools.

## Problem

AI agents spend significant tokens calling `hermes_search` or `hermes_lookup` just to locate symbols. A pre-loaded symbol summary in AGENTS.md eliminates these tool calls for common lookups.

## What to Build

- CLI command: `hermes inject-symbols [--path AGENTS.md] [--all] [--budget 2000]`
- Reads `symbol_index` joined with `blast_scores` for priority ordering
- Writes compressed format between HTML comment markers
- Idempotent: re-runs update in place
- Token-budgeted: stops when budget exhausted (high-blast files first)

## Acceptance Criteria

- [ ] CLI command `hermes inject-symbols` works
- [ ] Output format: `src/auth.rs: verify_token:fn:18 AuthService:st:44[login,logout]`
- [ ] Kind abbreviations: fn, st, en, tr, im, md, if, co
- [ ] Bounded by `<!-- hermes-symbols-start -->` / `<!-- hermes-symbols-end -->` markers
- [ ] Default: exported symbols only, `--all` includes private
- [ ] Prioritized by blast score (highest first)
- [ ] Respects token budget (default 2000)
- [ ] Idempotent re-runs

## Links

- [Implementation Plan](./plan.md)
