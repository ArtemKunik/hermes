# Spec: Blast-Radius Scoring Engine

## Data Model

### `blast_scores` Table

```sql
CREATE TABLE IF NOT EXISTS blast_scores (
    node_id          TEXT PRIMARY KEY REFERENCES nodes(id),
    project_id       TEXT NOT NULL,
    file_path        TEXT,
    direct_count     INTEGER NOT NULL DEFAULT 0,
    transitive_count INTEGER NOT NULL DEFAULT 0,
    blast_score      REAL NOT NULL DEFAULT 0.0,
    risk_level       TEXT NOT NULL DEFAULT 'LOW',
    computed_at      TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_blast_project ON blast_scores(project_id);
CREATE INDEX IF NOT EXISTS idx_blast_score_desc ON blast_scores(blast_score DESC);
CREATE INDEX IF NOT EXISTS idx_blast_file ON blast_scores(file_path);
```

### Supporting Index

```sql
CREATE INDEX IF NOT EXISTS idx_edges_project_type ON edges(project_id, edge_type);
```

## Algorithm

### BFS with Edge Filtering

1. For each node N in the project:
   - Initialize `visited = {N}`, `queue = [(N, depth=0)]`
   - `direct = 0`, `transitive = 0`
2. Dequeue `(current, depth)`:
   - If `depth >= 3`, skip expansion
   - Query incoming edges WHERE `target_id = current AND edge_type IN ('calls','imports','uses','depends_on','implements')`
   - For each unvisited upstream node: mark visited, enqueue at `depth + 1`
   - If `depth == 0`: increment `direct`
   - If `depth > 0`: increment `transitive`
3. Score = `direct + (0.5 × transitive)`
4. Risk level:
   - `total_nodes` = count of all project nodes
   - `affected = direct + transitive`
   - HIGH if `affected / total_nodes > 0.15`
   - MEDIUM if `affected / total_nodes > 0.05`
   - LOW otherwise

### Optimization

- Batch computation: single pass over all nodes, not N individual BFS queries
- Use in-memory adjacency list built from one `SELECT` query
- Upsert results in batch `INSERT OR REPLACE`

## MCP Tool Schemas

### `hermes_blast_score`

```json
{
  "name": "hermes_blast_score",
  "description": "Blast-radius report for a symbol or file. Returns score, risk level, and dependency breakdown.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "symbol_name": { "type": "string", "description": "Symbol name to look up (function, struct, etc.)" },
      "file_path": { "type": "string", "description": "File path to look up (alternative to symbol_name)" }
    }
  }
}
```

### `hermes_high_blast`

```json
{
  "name": "hermes_high_blast",
  "description": "List files/symbols with blast score above threshold, sorted by score descending.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "threshold": { "type": "number", "description": "Minimum blast score (default: 5.0)" },
      "limit": { "type": "integer", "description": "Max results (default: 20)" }
    }
  }
}
```

## Enhanced `hermes_impact_analysis` Output

Add to existing response:
```json
{
  "risk_level": "HIGH",
  "direct_dependents": 5,
  "transitive_dependents": 12,
  "codebase_percentage": 16.7
}
```

## Non-Goals

- Edge weight utilization (edge `weight` column stays at 1.0)
- Configurable depth limit (stays at 3)
- Real-time score updates on individual file changes (batch during indexing)
