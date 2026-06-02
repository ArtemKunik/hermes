# TRACK-068: Blast-Radius Scoring Engine

**Status**: completed
**Created**: 2026-05-29
**Type**: Feature / Graph Analysis
**Depends on**: None — can start immediately
**Branch**: `feat/blast-radius`

## Goal

Persist per-node blast-radius scores computed during indexing, exposing risk levels and high-blast listings via MCP tools. Enable AI agents to assess change impact before modifying files.

## Problem

`hermes_impact_analysis` performs on-demand BFS but:
1. Follows ALL edge types including `Contains` and `Documents` (not true dependencies)
2. Returns a flat count with no risk classification
3. No way to ask "what are the riskiest files?" without knowing which to query
4. Scores are recomputed from scratch on every call

## What to Build

- `blast_scores` table with persistent per-node scores
- BFS filtered to dependency edges: `Calls`, `Imports`, `Uses`, `DependsOn`, `Implements`
- Weighted formula: `direct + (0.5 × transitive)`
- Risk levels: HIGH (>15% of codebase), MEDIUM (>5%), LOW
- Auto-compute during indexing (after xref phase)
- 2 new MCP tools: `hermes_blast_score`, `hermes_high_blast`
- Enhance `hermes_impact_analysis` with edge filtering + risk output

## Acceptance Criteria

- [ ] `blast_scores` table created with migration
- [ ] Scores computed during `ingest_directory()` after xref extraction
- [ ] BFS follows only dependency edge types
- [ ] `hermes_blast_score` returns score, risk level, direct/transitive breakdown
- [ ] `hermes_high_blast` returns top-N files above threshold
- [ ] `hermes_impact_analysis` filters to dependency edges and includes risk_level
- [ ] All existing tests pass, new tests added for scoring formula
- [ ] Tool count test updated to 60

## Related Tracks

- TRACK-070: AGENTS.md Symbol Injection (uses blast scores for priority)
- TRACK-071: Pre-commit Hook (reads blast_scores for threshold checking)
- TRACK-073: Visualization UI (reads blast_scores for heatmap)

## Links

- [Specification](./spec.md)
- [Implementation Plan](./plan.md)
