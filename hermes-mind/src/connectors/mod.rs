pub mod gmail;
pub mod onedrive;
pub mod telegram;
pub mod whatsapp;

use anyhow::Result;
use hermes_engine::graph::KnowledgeGraph;

#[derive(Debug, Default)]
pub struct SyncReport {
    pub ingested: usize,
    pub skipped: usize,
    pub errors: usize,
}

pub trait Connector: Send + Sync {
    fn name(&self) -> &str;
    /// Pull new items and write them into the knowledge graph.
    fn sync(&self, graph: &KnowledgeGraph) -> Result<SyncReport>;
}
