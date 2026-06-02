# TRACK-073: Embedded Visualization UI

**Status**: completed
**Created**: 2026-05-29
**Type**: Feature / Developer Experience
**Depends on**: TRACK-068 (blast_scores for heatmap), TRACK-069 (symbol data)
**Branch**: `feat/viz-ui`

## Goal

Provide `hermes serve --viz` that launches an interactive browser-based visualization of the project's dependency graph, blast-radius heatmap, and module treemap.

## Problem

Hermes has rich graph data (nodes, edges, blast scores) but no visual way to explore it. Developers and architects must use CLI/MCP tools to understand project structure, making it hard to get a "big picture" view.

## What to Build

- CLI command: `hermes serve --viz [--port 8080]`
- Embedded HTTP server serving a single HTML file
- Three visualization modes: dependency graph, blast heatmap, module treemap
- JSON API endpoints for graph data
- d3.js loaded from CDN (no bundling)

## Acceptance Criteria

- [x] `hermes serve --viz` starts HTTP server on configurable port
- [x] Dependency graph: force-directed 2D, nodes colored by blast score
- [x] Blast heatmap: file tree with color intensity = blast score
- [x] Module treemap: area = LOC, color = blast score
- [x] Click node for detail panel (symbols, blast score, dependencies)
- [x] JSON API: `/api/graph`, `/api/blast`, `/api/symbols/:file`
- [x] Single HTML file, no build step required
- [x] Graceful shutdown on Ctrl+C

## Links

- [Specification](./spec.md)
- [Implementation Plan](./plan.md)
