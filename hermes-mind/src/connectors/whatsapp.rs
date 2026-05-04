// WhatsApp connector via a Baileys Node.js sidecar.
//
// The sidecar (tools/hermes-mind-wa/index.js) connects to WhatsApp Web,
// authenticates via QR code on first run (saves creds to HERMES_MIND_WA_CREDS_PATH),
// and streams newline-delimited JSON events to stdout:
//
//   {"type":"message","id":"ABC123","from":"+1234567890","chat":"group-name",
//    "body":"hello world","timestamp":1700000000}
//
// Required env vars:
//   HERMES_MIND_WA_SIDECAR_PATH  — path to index.js
//   HERMES_MIND_WA_CREDS_PATH    — directory for creds.json (created on first run)

use super::{Connector, SyncReport};
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

    fn sync(&self, graph: &KnowledgeGraph) -> Result<SyncReport> {
        let mut report = SyncReport::default();

        let mut child = Command::new("node")
            .arg(&self.sidecar_path)
            .arg("--creds")
            .arg(&self.creds_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;

        let stdout = child.stdout.take().expect("stdout is piped");
        let reader = BufReader::new(stdout);

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
            report.ingested += 1;
        }

        child.wait()?;
        Ok(report)
    }
}
