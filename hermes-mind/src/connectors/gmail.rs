// Gmail connector via Google OAuth2 + Gmail REST API.
//
// Required env vars:
//   HERMES_MIND_GMAIL_CLIENT_ID
//   HERMES_MIND_GMAIL_CLIENT_SECRET
//   HERMES_MIND_GMAIL_REFRESH_TOKEN   — from `hermes-mind auth gmail`
//
// Incremental: stores the Unix timestamp of the last synced message in
// connector_state and uses Gmail's `after:<ts>` query filter on subsequent runs.

use super::{Connector, SyncReport};
use crate::sync_state::SyncState;
use anyhow::{Context, Result};
use hermes_engine::graph::{KnowledgeGraph, Node, NodeType};
use serde::Deserialize;

const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GMAIL_BASE: &str = "https://gmail.googleapis.com/gmail/v1/users/me";

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
}

#[derive(Deserialize)]
struct MessageList {
    messages: Option<Vec<MessageRef>>,
}

#[derive(Deserialize)]
struct MessageRef {
    id: String,
}

#[derive(Deserialize)]
struct MessageDetail {
    id: String,
    snippet: Option<String>,
    #[serde(rename = "internalDate")]
    internal_date: Option<String>,
    payload: Option<Payload>,
}

#[derive(Deserialize)]
struct Payload {
    headers: Option<Vec<Header>>,
}

#[derive(Deserialize)]
struct Header {
    name: String,
    value: String,
}

pub struct GmailConnector {
    client_id: String,
    client_secret: String,
    refresh_token: String,
}

impl GmailConnector {
    pub fn new(
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
        refresh_token: impl Into<String>,
    ) -> Self {
        Self {
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            refresh_token: refresh_token.into(),
        }
    }

    fn access_token(&self) -> Result<String> {
        let resp: TokenResponse = ureq::post(TOKEN_URL)
            .send_form(&[
                ("client_id", self.client_id.as_str()),
                ("client_secret", self.client_secret.as_str()),
                ("refresh_token", self.refresh_token.as_str()),
                ("grant_type", "refresh_token"),
            ])?
            .into_json()
            .context("parsing token response")?;
        Ok(resp.access_token)
    }

    fn list_message_ids(&self, token: &str, query: &str) -> Result<Vec<String>> {
        let mut req = ureq::get(&format!("{GMAIL_BASE}/messages"))
            .set("Authorization", &format!("Bearer {token}"))
            .query("maxResults", "100");
        if !query.is_empty() {
            req = req.query("q", query);
        }
        let resp: MessageList = req.call()?.into_json()?;
        Ok(resp
            .messages
            .unwrap_or_default()
            .into_iter()
            .map(|m| m.id)
            .collect())
    }

    fn get_message(&self, token: &str, id: &str) -> Result<MessageDetail> {
        let detail: MessageDetail = ureq::get(&format!("{GMAIL_BASE}/messages/{id}"))
            .set("Authorization", &format!("Bearer {token}"))
            .query("format", "metadata")
            .query("metadataHeaders", "From")
            .query("metadataHeaders", "Subject")
            .call()?
            .into_json()?;
        Ok(detail)
    }
}

impl Connector for GmailConnector {
    fn name(&self) -> &str {
        "gmail"
    }

    fn sync(&self, graph: &KnowledgeGraph, state: &SyncState) -> Result<SyncReport> {
        let mut report = SyncReport::default();
        let token = self.access_token()?;

        let query = state
            .get("gmail", "last_message_ts")?
            .map(|ts| format!("after:{ts}"))
            .unwrap_or_default();

        let ids = self.list_message_ids(&token, &query)?;
        let mut latest_ts: u64 = state
            .get("gmail", "last_message_ts")?
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        for id in ids {
            let Ok(msg) = self.get_message(&token, &id) else {
                report.errors += 1;
                continue;
            };

            let headers = msg.payload.and_then(|p| p.headers).unwrap_or_default();
            let subject = headers
                .iter()
                .find(|h| h.name == "Subject")
                .map(|h| h.value.as_str())
                .unwrap_or("(no subject)")
                .to_string();
            let from = headers
                .iter()
                .find(|h| h.name == "From")
                .map(|h| h.value.as_str())
                .unwrap_or("unknown")
                .to_string();

            // internalDate is milliseconds since epoch
            let msg_ts = msg
                .internal_date
                .as_deref()
                .and_then(|s| s.parse::<u64>().ok())
                .map(|ms| ms / 1000)
                .unwrap_or(0);

            let node_id = format!("gmail::{}", msg.id);
            let snippet = msg.snippet.unwrap_or_default();
            let node = Node {
                id: node_id,
                project_id: graph.project_id().to_string(),
                name: format!("{from}: {subject}"),
                node_type: NodeType::Email,
                file_path: None,
                start_line: Some(msg_ts as i64),
                end_line: None,
                summary: Some(snippet.chars().take(200).collect()),
                content_hash: None,
            };
            graph.add_node(&node)?;
            graph.index_fts(&node, &format!("{subject} {from} {snippet}"))?;
            latest_ts = latest_ts.max(msg_ts);
            report.ingested += 1;
        }

        if latest_ts > 0 {
            state.set("gmail", "last_message_ts", &latest_ts.to_string())?;
        }
        Ok(report)
    }
}
