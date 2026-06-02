# Spec: OpenCode Plugin Enhancement

## Phase A — Expose All Existing Tools

### Current Plugin Tools (17)
hermes_search, hermes_fetch, hermes_recall, hermes_remember, hermes_write_decision, hermes_fact, hermes_facts, hermes_lint, hermes_repo_map, hermes_blast_score, hermes_high_blast, hermes_constraints, hermes_review, hermes_index, hermes_lookup, hermes_file_symbols, hermes_stats

### Tools to Add (45)

**Core (5):**
- hermes_backfill — Recompute missing content_tokens
- hermes_slow_loop_status — Background task status
- hermes_generate_digest — Daily digest of activity
- hermes_compact_sessions — Compact session history
- hermes_generate_weekly_brief — Weekly project brief

**Skill Candidates (3):**
- hermes_approve_skill_candidate — Approve a proposed skill
- hermes_reject_skill_candidate — Reject a proposed skill
- hermes_apply_proposal — Apply a pending proposal

**Impact & Misses (2):**
- hermes_impact_analysis — Pre-change blast-radius impact estimate
- hermes_search_misses — Post-mortem zero-result queries

**Missions (5):**
- hermes_mission_start — Create a mission
- hermes_mission_update — Update mission status/details
- hermes_mission_event — Log event to a mission
- hermes_mission_status — Get mission status
- hermes_mission_list — List all missions

**Quality (5):**
- hermes_quality_score — Module quality scores
- hermes_quality_next — Next open finding
- hermes_quality_resolve — Mark finding resolved
- hermes_quality_wontfix — Acknowledge (halve penalty)
- hermes_quality_dismiss — Dismiss a finding
- hermes_lint_dismiss — Dismiss a lint violation
- hermes_dismissed_list — List dismissed items
- hermes_auto_dismiss — Auto-dismiss stale findings

**Incidents & KB (5):**
- hermes_log_incident — Log a new incident
- hermes_resolve_incident — Resolve an incident
- hermes_query_incidents — Query incidents
- hermes_write_kb_article — Write knowledge base article
- hermes_search_kb — Search knowledge base

**Tracks (2):**
- hermes_list_tracks — List conductor tracks
- hermes_resume_track — Build resume brief for a track

**Skills (3):**
- hermes_tools — Intent-based tool listing
- hermes_match_skills — Match skills by query
- hermes_fetch_skill — Fetch a skill by path

**Coverage (1):**
- hermes_test_coverage_map — Map tests to code

**Validation (2):**
- hermes_validate_env — Check env var against registry
- hermes_validate_symbols — Check symbols exist in graph

**Commit (1):**
- hermes_prepare_commit_message — Generate commit message

**Memory (2):**
- hermes_memory_stats — Memory usage stats
- hermes_battery_check — Memory/recall battery level

**Status (1):**
- hermes_mcp_status — MCP server status

**Consistency (1):**
- hermes_check_consistency — Codebase consistency check

**Heal (1):**
- hermes_heal_violations — Auto-fix violations

**Memory Query (2):**
- hermes_query_memory — Direct memory store query
- hermes_get_core_facts — Core project facts

**Coverage (1):**
- hermes_test_coverage_map — Test-to-code coverage

### TypeScript Tool Template

```typescript
hermes_<name>: tool({
  description: "<description from Rust schema>",
  args: {
    <args from Rust schema, camelCased>
  },
  async execute(args) {
    return hermesCall("hermes_<name>", args)
  },
}),
```

## Phase B — New Capabilities

### B1: `hermes_viz_graph` MCP Tool

New Rust handler in `src/viz/api.rs` that queries nodes + edges + blast_scores and returns d3-compatible JSON. Wraps the `/api/graph` endpoint logic. Registered in `mcp_actor_dispatch.rs`.

**Returns:**
```json
{
  "nodes": [
    { "id": "src/auth.rs", "name": "auth.rs", "file": "src/auth.rs",
      "blast_score": 8.5, "risk": "HIGH", "loc": 142 }
  ],
  "edges": [
    { "source": "src/api.rs", "target": "src/auth.rs", "type": "imports" }
  ]
}
```

### B2: Blast-Score Write Guard

Enhance the `tool.execute.after` hook. On `write`/`edit`:
1. Extract modified file path from the tool output
2. Call `hermes_blast_score` on that path
3. If `risk_level === "HIGH"`, prepend warning to session context:
   ```
   ⚠️ Modified {file} — blast score {score} (HIGH risk).
   This file affects {direct_count} direct + {transitive_count} transitive dependents.
   ```

### B3: Enhanced Session Compaction Hook

Current: injects single recall query result. Proposed: also inject:
- Top-5 high-blast files summary
- Recent decisions from `write_decision`
- Unresolved findings count

## Phase C — AGENTS.md Auto-Injection

### On Session Start

New lifecycle hook (`session.start` or equivalent):
1. Check if `AGENTS.md` exists in project root
2. Run `hermes inject-symbols --budget 2000`
3. Log result to console

### Fallback
If `hermes inject-symbols` not available (ccterm unreachable), skip silently.

## Non-Goals

- Rewriting the Rust MCP registry — only adding the viz handler
- Modifying ccterm proxy layer
- Bundling d3.js locally (CDN still preferred)
- Authentication for the viz UI
