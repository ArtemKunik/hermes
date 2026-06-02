# Spec: Symbol Index + Fast Lookup

## Data Model

### `symbol_index` Table

```sql
CREATE TABLE IF NOT EXISTS symbol_index (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id  TEXT NOT NULL,
    name        TEXT NOT NULL,
    file_path   TEXT NOT NULL,
    line        INTEGER NOT NULL,
    kind        TEXT NOT NULL,
    exported    INTEGER NOT NULL DEFAULT 0,
    methods     TEXT,
    UNIQUE(project_id, name, file_path, line)
);
CREATE INDEX IF NOT EXISTS idx_sym_name ON symbol_index(name);
CREATE INDEX IF NOT EXISTS idx_sym_file ON symbol_index(project_id, file_path);
```

## Population Logic

### During Ingestion (`file_ops.rs`)

After creating each symbol node, also insert into `symbol_index`:

| Source | `name` | `line` | `kind` | `exported` | `methods` |
|--------|--------|--------|--------|------------|-----------|
| AST chunker | `extract_node_name()` | `node.start_position().row + 1` | `object_type` field | Check for `pub` in source text | For `impl_item`: parse method names |
| Regex chunker | Captured group from regex | Line number from chunk | Mapped from regex pattern | Check for `pub`/`export` prefix | For `impl`: parse `fn` names |

### Cleanup

On re-index: `DELETE FROM symbol_index WHERE project_id = ? AND file_path = ?` before re-ingestion (matches existing `delete_nodes_for_file()` pattern).

## MCP Tool Schemas

### `hermes_lookup`

```json
{
  "name": "hermes_lookup",
  "description": "O(1) symbol lookup: find where a function, struct, class, or type is defined. Returns file, line, kind, and exported flag.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "symbol_name": { "type": "string", "description": "Symbol name to find (e.g. 'verify_token', 'AuthService')" }
    },
    "required": ["symbol_name"]
  }
}
```

### `hermes_file_symbols`

```json
{
  "name": "hermes_file_symbols",
  "description": "List all symbols defined in a file with their line numbers, kinds, and exported status.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "file_path": { "type": "string", "description": "File path relative to project root" }
    },
    "required": ["file_path"]
  }
}
```

## Non-Goals

- Fuzzy/prefix matching (use `hermes_search` for that)
- Symbol usage/reference tracking (use `hermes_impact_analysis`)
- Cross-project symbol lookup
