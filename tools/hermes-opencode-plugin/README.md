# Hermes Opencode Plugin

Bridges [Hermes](https://github.com/anomalyco/opencode) knowledge-engine tools into
opencode sessions via the ccterm HTTP proxy running on `:38080`.

## Install

```bash
mkdir -p ~/.config/opencode/plugins
cp hermes.ts ~/.config/opencode/plugins/
```

Or symlink for auto-updates:

```bash
ln -s "$PWD/hermes.ts" ~/.config/opencode/plugins/hermes.ts
```

Requires `@opencode-ai/plugin` in `~/.config/opencode/package.json`:

```json
{
  "type": "module",
  "dependencies": {
    "@opencode-ai/plugin": "^1.15.10"
  }
}
```

## Prerequisites

- ccterm running with Hermes proxy enabled (default port `38080`)
- `CCTERM_USERNAME` / `CCTERM_PASSWORD` environment variables (or `VIBETUNNEL_RUST_*`)

## Tools registered

| Tool | Bridges to | Use case |
|------|-----------|---------|
| `hermes_search` | `hermes_search` | Pointer-RAG code search |
| `hermes_fetch` | `hermes_fetch` | Full content by node ID |
| `hermes_recall` | `hermes_recall` | Prior session memory |
| `hermes_remember` | `hermes_remember` | Save session summaries |
| `hermes_write_decision` | `hermes_write_decision` | Decision docs |
| `hermes_fact` | `hermes_fact` | Temporal fact storage |
| `hermes_facts` | `hermes_facts` | List active facts |
| `hermes_lint` | `hermes_lint_architecture` | Architecture rules |
| `hermes_repo_map` | `hermes_repo_map` | Compact symbol overview |
| `hermes_constraints` | `hermes_constraints` | File-level constraints |
| `hermes_review` | `hermes_quality_review` | LLM code review |
| `hermes_index` | `hermes_index` | Trigger re-index |
| `hermes_stats` | `hermes_stats` | Token savings |

## Hooks

- **Auto-index**: 5s debounced re-index after `bash` / `write` / `edit` tool calls
- **Compaction**: Injects Hermes recall context into session compaction
