# TRACK-071: Pre-commit Hook

**Status**: completed
**Created**: 2026-05-29
**Type**: Feature / Developer Experience
**Depends on**: TRACK-068 (blast_scores table)
**Branch**: `feat/precommit-hook`

## Goal

Provide a CLI command that installs a git pre-commit hook warning developers when staged files have high blast-radius scores.

## Problem

Developers (human and AI) modify high-blast files without awareness of the downstream impact. A pre-commit warning creates a safety net.

## What to Build

- CLI command: `hermes install-hook [--threshold 10] [--strict] [--remove]`
- Generates `.git/hooks/pre-commit` shell script
- Script reads blast_scores from SQLite DB for staged files
- Warns (or blocks with `--strict`) when files exceed threshold

## Acceptance Criteria

- [ ] `hermes install-hook` creates `.git/hooks/pre-commit`
- [ ] Hook reads blast scores from hermes DB
- [ ] Warns when staged files exceed threshold (default: 10)
- [ ] `--strict` mode blocks commit (exit 1)
- [ ] `--remove` deletes the hook
- [ ] Hook is idempotent (re-install overwrites cleanly)

## Links

- [Implementation Plan](./plan.md)
