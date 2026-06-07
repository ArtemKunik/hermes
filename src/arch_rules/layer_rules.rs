// tools/hermes-engine/src/arch_rules/layer_rules.rs
// TRACK-045: LAYER-001 through LAYER-005 — architectural layer boundary rules.

use anyhow::Result;
use rusqlite::params;
use std::path::Path;

use crate::arch_rules::{ArchRule, Severity, Violation};
use crate::graph::KnowledgeGraph;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn path_contains(path: &str, segment: &str) -> bool {
    let norm = path.replace('\\', "/");
    norm.contains(segment)
}

// ---------------------------------------------------------------------------
// LAYER-001: Handler imports store directly
// ---------------------------------------------------------------------------

pub struct LayerHandlerImportsStore;

impl ArchRule for LayerHandlerImportsStore {
    fn id(&self) -> &str {
        "LAYER-001"
    }
    fn severity(&self) -> Severity {
        Severity::Error
    }
    fn description(&self) -> &str {
        "Handler module imports a store module directly (skips service layer)"
    }

    fn evaluate(&self, graph: &KnowledgeGraph, _root: &Path) -> Result<Vec<Violation>> {
        let conn = graph.db().lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut stmt = conn.prepare(
            "SELECT DISTINCT n.file_path, n.name, t.file_path
             FROM nodes n
             JOIN edges e ON e.source_id = n.id
             JOIN nodes t ON t.id = e.target_id
             WHERE n.project_id = ?1 AND e.edge_type = 'imports'
               AND (n.file_path LIKE '%/handlers/%' OR n.file_path LIKE '%\\handlers\\%'
                    OR n.file_path LIKE '%/handler/%' OR n.file_path LIKE '%\\handler\\%')
               AND (t.file_path LIKE '%store_%' OR t.file_path LIKE '%_store%'
                    OR t.file_path LIKE '%/store/%' OR t.file_path LIKE '%\\store\\%'
                    OR t.file_path LIKE '%cosmos_%')",
        )?;
        let rows: Vec<(String, String, String)> = stmt
            .query_map(params![graph.project_id()], |row| {
                Ok((
                    row.get::<_, String>(0).unwrap_or_default(),
                    row.get::<_, String>(1).unwrap_or_default(),
                    row.get::<_, String>(2).unwrap_or_default(),
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);
        drop(conn);

        Ok(rows
            .into_iter()
            .map(|(fp, sym, target)| {
                Violation::new(self.id(), self.severity(), &fp,
                    format!("Handler `{sym}` imports store `{target}` directly — route through a service"))
                    .with_symbol(&sym)
                    .with_suggestion("Extract business logic into a service; have the handler call the service only")
            })
            .collect())
    }
}

// ---------------------------------------------------------------------------
// LAYER-002: Handler contains business logic (function span > 30 lines)
// ---------------------------------------------------------------------------

pub struct LayerHandlerBusinessLogic;

impl ArchRule for LayerHandlerBusinessLogic {
    fn id(&self) -> &str {
        "LAYER-002"
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }
    fn description(&self) -> &str {
        "Handler function body exceeds 30 lines (likely contains business logic)"
    }

    fn evaluate(&self, graph: &KnowledgeGraph, _root: &Path) -> Result<Vec<Violation>> {
        let conn = graph.db().lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut stmt = conn.prepare(
            "SELECT name, file_path, start_line, end_line
             FROM nodes
             WHERE project_id = ?1
               AND node_type IN ('function', 'impl')
               AND (file_path LIKE '%/handlers/%' OR file_path LIKE '%\\handlers\\%'
                    OR file_path LIKE '%/handler/%' OR file_path LIKE '%\\handler\\%')
               AND end_line IS NOT NULL AND start_line IS NOT NULL
               AND (end_line - start_line) > 30",
        )?;
        let rows: Vec<(String, String, i64, i64)> = stmt
            .query_map(params![graph.project_id()], |row| {
                Ok((
                    row.get::<_, String>(0).unwrap_or_default(),
                    row.get::<_, String>(1).unwrap_or_default(),
                    row.get::<_, i64>(2).unwrap_or(0),
                    row.get::<_, i64>(3).unwrap_or(0),
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);
        drop(conn);

        Ok(rows
            .into_iter()
            .map(|(sym, fp, start, end)| {
                let lines = end - start;
                Violation::new(self.id(), self.severity(), &fp,
                    format!("`{sym}` in handler spans {lines} lines — handlers must be thin (≤30 lines)"))
                    .with_lines(start as u32, end as u32)
                    .with_symbol(&sym)
                    .with_suggestion("Move business logic to a service function")
            })
            .collect())
    }
}

// ---------------------------------------------------------------------------
// LAYER-003: Service imports handler module
// ---------------------------------------------------------------------------

pub struct LayerServiceImportsHandler;

impl ArchRule for LayerServiceImportsHandler {
    fn id(&self) -> &str {
        "LAYER-003"
    }
    fn severity(&self) -> Severity {
        Severity::Error
    }
    fn description(&self) -> &str {
        "Service module imports a handler module (inverted dependency)"
    }

    fn evaluate(&self, graph: &KnowledgeGraph, _root: &Path) -> Result<Vec<Violation>> {
        let conn = graph.db().lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut stmt = conn.prepare(
            "SELECT DISTINCT n.file_path, n.name, t.file_path
             FROM nodes n
             JOIN edges e ON e.source_id = n.id
             JOIN nodes t ON t.id = e.target_id
             WHERE n.project_id = ?1 AND e.edge_type = 'imports'
               AND (n.file_path LIKE '%_service%' OR n.file_path LIKE '%/service/%'
                    OR n.file_path LIKE '%\\service\\%')
               AND (t.file_path LIKE '%/handlers/%' OR t.file_path LIKE '%\\handlers\\%'
                    OR t.file_path LIKE '%/handler/%' OR t.file_path LIKE '%\\handler\\%')",
        )?;
        let rows: Vec<(String, String, String)> = stmt
            .query_map(params![graph.project_id()], |row| {
                Ok((
                    row.get::<_, String>(0).unwrap_or_default(),
                    row.get::<_, String>(1).unwrap_or_default(),
                    row.get::<_, String>(2).unwrap_or_default(),
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);
        drop(conn);

        Ok(rows
            .into_iter()
            .map(|(fp, sym, target)| {
                Violation::new(self.id(), self.severity(), &fp,
                    format!("Service `{sym}` imports handler `{target}` — services must not depend on handlers"))
                    .with_symbol(&sym)
                    .with_suggestion("Remove the handler import; handlers call services, not the reverse")
            })
            .collect())
    }
}

// ---------------------------------------------------------------------------
// LAYER-004: React component calls fetch/axios/API directly
// ---------------------------------------------------------------------------

pub struct LayerComponentFetch;

impl ArchRule for LayerComponentFetch {
    fn id(&self) -> &str {
        "LAYER-004"
    }
    fn severity(&self) -> Severity {
        Severity::Error
    }
    fn description(&self) -> &str {
        "React component makes direct fetch/axios/API call (should use hook or service)"
    }

    fn evaluate(&self, graph: &KnowledgeGraph, _root: &Path) -> Result<Vec<Violation>> {
        let conn = graph.db().lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut stmt = conn.prepare(
            "SELECT DISTINCT n.file_path, n.name, t.name
             FROM nodes n
             JOIN edges e ON e.source_id = n.id
             JOIN nodes t ON t.id = e.target_id
             WHERE n.project_id = ?1 AND e.edge_type = 'calls'
               AND (n.file_path LIKE '%/components/%' OR n.file_path LIKE '%\\components\\%')
               AND (LOWER(t.name) IN ('fetch', 'axios', 'get', 'post', 'put', 'delete')
                    OR t.name LIKE 'api.%' OR t.name LIKE 'axios.%')",
        )?;
        let rows: Vec<(String, String, String)> = stmt
            .query_map(params![graph.project_id()], |row| {
                Ok((
                    row.get::<_, String>(0).unwrap_or_default(),
                    row.get::<_, String>(1).unwrap_or_default(),
                    row.get::<_, String>(2).unwrap_or_default(),
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);
        drop(conn);

        Ok(rows
            .into_iter()
            .map(|(fp, sym, callee)| {
                Violation::new(
                    self.id(),
                    self.severity(),
                    &fp,
                    format!(
                        "Component `{sym}` calls `{callee}` directly — move to a hook or service"
                    ),
                )
                .with_symbol(&sym)
                .with_suggestion(
                    "Extract the API call into a custom hook (useXxx) or service function",
                )
            })
            .collect())
    }
}

// ---------------------------------------------------------------------------
// LAYER-005: Component imports from services/api directly
// ---------------------------------------------------------------------------

pub struct LayerComponentImportsApi;

impl ArchRule for LayerComponentImportsApi {
    fn id(&self) -> &str {
        "LAYER-005"
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }
    fn description(&self) -> &str {
        "React component imports from services/api directly (should go through a hook)"
    }

    fn evaluate(&self, graph: &KnowledgeGraph, _root: &Path) -> Result<Vec<Violation>> {
        let conn = graph.db().lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut stmt = conn.prepare(
            "SELECT DISTINCT n.file_path, n.name, t.file_path
             FROM nodes n
             JOIN edges e ON e.source_id = n.id
             JOIN nodes t ON t.id = e.target_id
             WHERE n.project_id = ?1 AND e.edge_type = 'imports'
               AND (n.file_path LIKE '%/components/%' OR n.file_path LIKE '%\\components\\%')
               AND (t.file_path LIKE '%services/api%' OR t.file_path LIKE '%services\\api%'
                    OR t.file_path LIKE '%/api/%' OR t.file_path LIKE '%\\api\\%')",
        )?;
        let rows: Vec<(String, String, String)> = stmt
            .query_map(params![graph.project_id()], |row| {
                Ok((
                    row.get::<_, String>(0).unwrap_or_default(),
                    row.get::<_, String>(1).unwrap_or_default(),
                    row.get::<_, String>(2).unwrap_or_default(),
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);
        drop(conn);

        Ok(rows
            .into_iter()
            .filter(|(fp, _, _)| !path_contains(fp, "/hooks/") && !path_contains(fp, "\\hooks\\"))
            .map(|(fp, sym, target)| {
                Violation::new(
                    self.id(),
                    self.severity(),
                    &fp,
                    format!(
                        "Component `{sym}` imports `{target}` directly — use a custom hook instead"
                    ),
                )
                .with_symbol(&sym)
                .with_suggestion(
                    "Wrap the API import in a useXxx hook and call the hook from the component",
                )
            })
            .collect())
    }
}
