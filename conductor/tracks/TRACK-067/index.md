# TRACK-067: Fix stale search results after file deletion

**Status**: planned

## Problem

When files are deleted from disk, their indexed entries persist in Hermes search results even after re-indexing. This affects both code search and memory recall.

## Root Cause

`cleanup_stale_nodes()` (src/ingestion/mod.rs:182) calls `get_all_file_paths()` which only returns paths from `node_type='file'` nodes. Orphaned Function/Class/Method/etc. nodes with the same file_path are invisible to cleanup, so their entries survive deletion and re-index cycles.

## Impact

- Deleted files continue appearing in search results
- Manual `hermes delete-node` on one node leaves behind related nodes from the same file
- Memory indexer (Qdrant) never removes points for deleted .md files
- Background ingestion after `remember`/`write_decision` doesn't invalidate search cache
