# TRACK-074: OpenCode Plugin Enhancement

**Status**: completed
**Created**: 2026-05-29
**Type**: Feature / Developer Experience
**Depends on**: TRACK-068 (blast_scores), TRACK-069 (symbol_index), TRACK-072 (multi-lang AST), TRACK-073 (viz UI)
**Branch**: `feat/opencode-plugin`

## Goal

Upgrade the Hermes OpenCode TypeScript plugin from 17 tools to full coverage (~62 tools), add lifecycle hooks (blast-score write guard, AGENTS.md injection, session-start injection), and expose viz graph data as MCP tools.

## Problem

The plugin currently exposes only 17 of 62 Rust MCP tools. AI agents miss: quality workflows (resolve, wontfix, score), mission tracking, incident/KB management, heal_violations, test coverage maps, impact analysis, and more. The hooks are minimal — auto-index only. No blast-score awareness, no AGENTS.md auto-injection, no pre-session symbol warmup.

## What to Build

- **Phase A**: TypeScript wrappers for all ~45 unexposed tools
- **Phase B**: New Rust handler `hermes_viz_graph` + blast-score write guard hook
- **Phase C**: AGENTS.md auto-injection on session start

## Acceptance Criteria

- [ ] All 62 Rust MCP tools have corresponding TypeScript wrappers in `hermes.ts`
- [ ] `hermes_viz_graph` MCP tool returns d3-compatible `{nodes, edges}` JSON
- [ ] Write/edit hook checks blast score of affected files, warns if HIGH risk
- [ ] Session compaction hook injects high-blast file summaries
- [ ] AGENTS.md symbol table auto-injected on session start
- [ ] README updated with complete tool table
- [ ] All existing tests pass

## Links

- [Specification](./spec.md)
- [Implementation Plan](./plan.md)
