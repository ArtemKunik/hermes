# Plan: TRACK-074 — OpenCode Plugin Enhancement

## Phase A — Expose Existing Tools

- [ ] Add ~45 TypeScript tool wrappers to `tools/hermes-opencode-plugin/hermes.ts`
- [ ] Group tools by category (core, quality, missions, incidents, etc.)
- [ ] Update README tool table with all 62 tools
- [ ] Verify plugin loads in OpenCode without errors

## Phase B — New Rust Handler + Hooks

- [ ] Add `hermes_viz_graph` handler in `src/viz/api.rs` (export function, registered in mcp_actor_dispatch)
- [ ] Create `hermes_viz_graph` TypeScript wrapper
- [ ] Implement blast-score write guard in `tool.execute.after` hook
- [ ] Enhance session compaction hook with blast summary + recent decisions
- [ ] Tests for `hermes_viz_graph` handler

## Phase C — Session Start Injection

- [ ] Add `session.start` lifecycle hook (or equivalent)
- [ ] Implement `hermes inject-symbols --budget 2000` call on session start
- [ ] Graceful fallback on ccterm unreachable

## Files to Touch

| File | Change | Risk |
|------|--------|------|
| `tools/hermes-opencode-plugin/hermes.ts` | Add ~45 tool wrappers, enhance hooks | Medium |
| `tools/hermes-opencode-plugin/README.md` | Update tool table | Low |
| `src/viz/api.rs` | Add `hermes_viz_graph` handler function | Low |
| `src/mcp_actor_dispatch.rs` | Register `hermes_viz_graph` tool | Low |
