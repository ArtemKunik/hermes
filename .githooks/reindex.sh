#!/usr/bin/env sh
set -eu

# Run from repository root regardless of caller location.
repo_root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
cd "$repo_root"

# Allow opting out for CI or one-off local workflows.
if [ "${HERMES_SKIP_HOOK_REINDEX:-0}" = "1" ]; then
  exit 0
fi

echo "[hermes-hooks] Reindexing workspace..."

# Prefer installed binary; fall back to cargo run in development clones.
if command -v hermes >/dev/null 2>&1; then
  HERMES_PROJECT_ROOT="$repo_root" hermes index >/dev/null
else
  HERMES_PROJECT_ROOT="$repo_root" cargo run --quiet --bin hermes -- index >/dev/null
fi

echo "[hermes-hooks] Reindex complete."
