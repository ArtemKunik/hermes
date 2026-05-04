// Telegram connector via Bot API long polling.
//
// Required env vars:
//   HERMES_MIND_TELEGRAM_BOT_TOKEN  — from @BotFather
//
// Incremental: persists the getUpdates offset in connector_state so restarts
// don't replay already-processed updates.

use super::{Connector, SyncReport};
use crate::sync_state::SyncState;
use anyhow::Result;
use hermes_engine::graph::{KnowledgeGraph, Node, NodeType};
use serde::Deserialize;

const API_BASE: &str = "https://api.telegram.org";

#[derive(Deserialize)]
struct UpdateList {
    result: Vec<Update>,
}

#[derive(Deserialize)]
struct Update {
    update_id: i64,
    message: Option<Message>,
}

#[derive(Deserialize)]
struct Message {
    message_id: i64,
    text: Option<String>,
    chat: Chat,
    from: Option<From>,
    date: i64,
}

#[derive(Deserialize)]
struct Chat {
    id: i64,
    title: Option<String>,
    username: Option<String>,
}

#[derive(Deserialize)]
struct From {
    username: Option<String>,
    first_name: String,
}

pub struct TelegramConnector {
    token: String,
}

impl TelegramConnector {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
        }
    }

    fn get_updates(&self, offset: i64) -> Result<Vec<Update>> {
        let url = format!("{API_BASE}/bot{}/getUpdates", self.token);
        let resp: UpdateList = ureq::get(&url)
            .query("timeout", "10")
            .query("offset", &offset.to_string())
            .call()?
            .into_json()?;
        Ok(resp.result)
    }
}

impl Connector for TelegramConnector {
    fn name(&self) -> &str {
        "telegram"
    }

    fn sync(&self, graph: &KnowledgeGraph, state: &SyncState) -> Result<SyncReport> {
        let mut report = SyncReport::default();

        let mut offset = state
            .get("telegram", "offset")?
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);

        let updates = self.get_updates(offset)?;

        for update in updates {
            let next = update.update_id + 1;

            let Some(msg) = update.message else {
                offset = offset.max(next);
                continue;
            };

            let Some(text) = msg.text else {
                report.skipped += 1;
                offset = offset.max(next);
                continue;
            };

            let chat_name = msg
                .chat
                .title
                .or(msg.chat.username)
                .unwrap_or_else(|| msg.chat.id.to_string());
            let sender = msg
                .from
                .map(|f| f.username.unwrap_or(f.first_name))
                .unwrap_or_else(|| "unknown".to_string());

            let node_id = format!("tg::{}::{}", msg.chat.id, msg.message_id);
            let node = Node {
                id: node_id,
                project_id: graph.project_id().to_string(),
                name: format!("{sender} @ {chat_name}"),
                node_type: NodeType::Message,
                file_path: None,
                start_line: Some(msg.date),
                end_line: None,
                summary: Some(text.chars().take(140).collect()),
                content_hash: None,
            };
            graph.add_node(&node)?;
            graph.index_fts(&node, &text)?;
            offset = offset.max(next);
            report.ingested += 1;
        }

        state.set("telegram", "offset", &offset.to_string())?;
        Ok(report)
    }
}
