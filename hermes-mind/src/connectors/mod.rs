pub mod gmail;
pub mod onedrive;
pub mod telegram;
pub mod whatsapp;

use anyhow::Result;
use hermes_engine::graph::KnowledgeGraph;

use crate::sync_state::SyncState;

#[derive(Debug, Default)]
pub struct SyncReport {
    pub ingested: usize,
    pub skipped: usize,
    pub errors: usize,
}

pub trait Connector: Send + Sync {
    fn name(&self) -> &str;
    /// Pull only new items since the last sync and write them into the graph.
    fn sync(&self, graph: &KnowledgeGraph, state: &SyncState) -> Result<SyncReport>;
}
