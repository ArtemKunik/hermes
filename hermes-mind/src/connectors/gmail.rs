// Gmail connector via Google OAuth2 + Gmail REST API.
//
// Required env vars:
//   HERMES_MIND_GMAIL_CLIENT_ID
//   HERMES_MIND_GMAIL_CLIENT_SECRET
//   HERMES_MIND_GMAIL_REFRESH_TOKEN   — obtained via OAuth2 consent flow (one-time)
//
// Fetches the 100 most recent threads and indexes subject + snippet as Email nodes.
// Full body fetch is deferred to hermes mind_fetch.

use super::{Connector, SyncReport};
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

    fn list_message_ids(&self, token: &str) -> Result<Vec<String>> {
        let resp: MessageList = ureq::get(&format!("{GMAIL_BASE}/messages"))
            .set("Authorization", &format!("Bearer {token}"))
            .query("maxResults", "100")
            .call()?
            .into_json()?;
        Ok(resp
            .messages
            .unwrap_or_default()
            .into_iter()
            .map(|m| m.id)
            .collect())
    }

    fn get_message(&self, token: &str, id: &str) -> Result<MessageDetail> {
        let detail: MessageDetail =
            ureq::get(&format!("{GMAIL_BASE}/messages/{id}"))
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

    fn sync(&self, graph: &KnowledgeGraph) -> Result<SyncReport> {
        let mut report = SyncReport::default();
        let token = self.access_token()?;
        let ids = self.list_message_ids(&token)?;

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

            let node_id = format!("gmail::{}", msg.id);
            let snippet = msg.snippet.unwrap_or_default();
            let node = Node {
                id: node_id,
                project_id: graph.project_id().to_string(),
                name: format!("{from}: {subject}"),
                node_type: NodeType::Email,
                file_path: None,
                start_line: None,
                end_line: None,
                summary: Some(snippet.chars().take(200).collect()),
                content_hash: None,
            };
            graph.add_node(&node)?;
            graph.index_fts(&node, &format!("{subject} {snippet}"))?;
            report.ingested += 1;
        }

        Ok(report)
    }
}
