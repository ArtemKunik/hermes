# Spec: Embedded Visualization UI

## Architecture

```
hermes serve --viz [--port 8080]
  |
  +-- HTTP server (tiny_http or std::net::TcpListener)
  |     GET /                -> embedded index.html
  |     GET /api/graph       -> JSON {nodes, edges}
  |     GET /api/blast       -> JSON [{file, score, risk}]
  |     GET /api/symbols/:f  -> JSON [{name, line, kind}]
  |
  +-- Single HTML file with embedded CSS + JS
        d3.js v7 from CDN
        Three tab-switched visualization modes
```

## API Endpoints

### `GET /api/graph`

```json
{
  "nodes": [
    { "id": "src/auth.rs", "file": "src/auth.rs", "blast_score": 8.5, "risk": "HIGH", "loc": 142, "symbols": 5 }
  ],
  "edges": [
    { "source": "src/api.rs", "target": "src/auth.rs", "type": "imports" }
  ]
}
```

### `GET /api/blast?threshold=5`

```json
[
  { "file_path": "src/db.rs", "blast_score": 13.0, "risk_level": "HIGH", "direct_count": 12, "transitive_count": 2 }
]
```

### `GET /api/symbols/:file`

```json
[
  { "name": "verify_token", "line": 18, "kind": "function", "exported": true }
]
```

## Visualization Modes

### 1. Dependency Graph
- d3 force-directed layout
- Nodes = files, sized by LOC
- Node color: green (LOW) → yellow (MEDIUM) → red (HIGH) blast score
- Edges = dependency relationships (Calls, Imports)
- Click node → side panel with symbols + blast details
- Zoom/pan support

### 2. Blast Heatmap
- File tree layout (d3 tree)
- Directory nodes aggregate child blast scores
- Color intensity = max blast score in subtree
- Hover shows file path + score + risk level

### 3. Module Treemap
- d3 treemap layout
- Rectangle area = file LOC
- Rectangle color = blast score (same green→red scale)
- Click rectangle → zoom into directory
- Labels show file name + score

## Dependencies

- `tiny_http` crate (add to Cargo.toml) — lightweight, sync, no async runtime needed
- d3.js v7 from `https://d3js.org/d3.v7.min.js` (CDN)

## Non-Goals

- Real-time updates (manual refresh)
- Authentication/authorization
- Editing or mutation from the UI
- Mobile-responsive design
