// WhatsApp connector via a Baileys Node.js sidecar.
//
// Required env vars:
//   HERMES_MIND_WA_SIDECAR_PATH  — path to tools/hermes-mind-wa/index.js
//   HERMES_MIND_WA_CREDS_PATH    — directory for creds.json (created on first run)
//
// Passes --since <unix_ts> to the sidecar so only messages newer than the
// last sync are streamed. Persists the timestamp in connector_state.

use super::{Connector, SyncReport};
use crate::sync_state::SyncState;
use anyhow::Result;
use hermes_engine::graph::{KnowledgeGraph, Node, NodeType};
use serde::Deserialize;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

#[derive(Deserialize)]
struct WaEvent {
    id: String,
    from: String,
    chat: String,
    body: String,
    timestamp: i64,
}

pub struct WhatsAppConnector {
    sidecar_path: String,
    creds_path: String,
}

impl WhatsAppConnector {
    pub fn new(sidecar_path: impl Into<String>, creds_path: impl Into<String>) -> Self {
        Self {
            sidecar_path: sidecar_path.into(),
            creds_path: creds_path.into(),
        }
    }
}

impl Connector for WhatsAppConnector {
    fn name(&self) -> &str {
        "whatsapp"
    }

    fn sync(&self, graph: &KnowledgeGraph, state: &SyncState) -> Result<SyncReport> {
        let mut report = SyncReport::default();

        let since = state
            .get("whatsapp", "last_ts")?
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);

        let mut cmd = Command::new("node");
        cmd.arg(&self.sidecar_path)
            .arg("--creds")
            .arg(&self.creds_path);
        if since > 0 {
            cmd.arg("--since").arg(since.to_string());
        }

        let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn()?;
        let stdout = child.stdout.take().expect("stdout is piped");
        let reader = BufReader::new(stdout);
        let mut latest_ts = since;

        for line in reader.lines() {
            let line = line?;
            let Ok(event) = serde_json::from_str::<WaEvent>(&line) else {
                report.errors += 1;
                continue;
            };

            let node_id = format!("wa::{}", event.id);
            let node = Node {
                id: node_id,
                project_id: graph.project_id().to_string(),
                name: format!("{} @ {}", event.from, event.chat),
                node_type: NodeType::Message,
                file_path: None,
                start_line: Some(event.timestamp),
                end_line: None,
                summary: Some(event.body.chars().take(140).collect()),
                content_hash: None,
            };
            graph.add_node(&node)?;
            graph.index_fts(&node, &event.body)?;
            latest_ts = latest_ts.max(event.timestamp);
            report.ingested += 1;
        }

        child.wait()?;
        state.set("whatsapp", "last_ts", &latest_ts.to_string())?;
        Ok(report)
    }
}
