// Telegram connector via Bot API long polling.
//
// Required env vars:
//   HERMES_MIND_TELEGRAM_BOT_TOKEN  — from @BotFather
//
// The bot must be added to the chats/groups you want indexed.
// Polls getUpdates in a tight loop; each message becomes a Message node.

use super::{Connector, SyncReport};
use anyhow::Result;
use hermes_engine::graph::{KnowledgeGraph, Node, NodeType};
use serde::Deserialize;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

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
    offset: Arc<AtomicI64>,
}

impl TelegramConnector {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            offset: Arc::new(AtomicI64::new(0)),
        }
    }

    fn get_updates(&self) -> Result<Vec<Update>> {
        let offset = self.offset.load(Ordering::Relaxed);
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

    fn sync(&self, graph: &KnowledgeGraph) -> Result<SyncReport> {
        let mut report = SyncReport::default();
        let updates = self.get_updates()?;

        for update in updates {
            let Some(msg) = update.message else {
                self.offset
                    .fetch_max(update.update_id + 1, Ordering::Relaxed);
                continue;
            };

            let Some(text) = msg.text else {
                report.skipped += 1;
                self.offset
                    .fetch_max(update.update_id + 1, Ordering::Relaxed);
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
            self.offset
                .fetch_max(update.update_id + 1, Ordering::Relaxed);
            report.ingested += 1;
        }

        Ok(report)
    }
}
