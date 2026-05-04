// OneDrive connector via Microsoft Graph API + OAuth2.
//
// Required env vars:
//   HERMES_MIND_ONEDRIVE_CLIENT_ID
//   HERMES_MIND_ONEDRIVE_CLIENT_SECRET
//   HERMES_MIND_ONEDRIVE_REFRESH_TOKEN  — from `hermes-mind auth onedrive`
//
// Incremental: stores the Graph delta link in connector_state after the first
// full sync. Subsequent runs call the delta link directly to get only changes.

use super::{Connector, SyncReport};
use crate::sync_state::SyncState;
use anyhow::{Context, Result};
use hermes_engine::graph::{KnowledgeGraph, Node, NodeType};
use serde::Deserialize;

const TOKEN_URL: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/token";
const GRAPH_BASE: &str = "https://graph.microsoft.com/v1.0/me/drive";

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
}

#[derive(Deserialize)]
struct DriveItemList {
    value: Vec<DriveItem>,
    #[serde(rename = "@odata.deltaLink")]
    delta_link: Option<String>,
    #[serde(rename = "@odata.nextLink")]
    next_link: Option<String>,
}

#[derive(Deserialize)]
struct DriveItem {
    id: String,
    name: String,
    #[serde(rename = "webUrl")]
    web_url: Option<String>,
    description: Option<String>,
    file: Option<serde_json::Value>,
    deleted: Option<serde_json::Value>,
}

pub struct OneDriveConnector {
    client_id: String,
    client_secret: String,
    refresh_token: String,
}

impl OneDriveConnector {
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
                ("scope", "Files.Read offline_access"),
            ])?
            .into_json()
            .context("parsing token response")?;
        Ok(resp.access_token)
    }

    /// Fetches pages following next_link until a delta_link is returned.
    fn fetch_pages(&self, token: &str, start_url: &str) -> Result<(Vec<DriveItem>, Option<String>)> {
        let mut all_items = Vec::new();
        let mut url = start_url.to_string();

        loop {
            let page: DriveItemList = ureq::get(&url)
                .set("Authorization", &format!("Bearer {token}"))
                .call()?
                .into_json()?;

            all_items.extend(page.value);

            match (page.next_link, page.delta_link) {
                (Some(next), _) => url = next,
                (None, delta) => return Ok((all_items, delta)),
            }
        }
    }
}

impl Connector for OneDriveConnector {
    fn name(&self) -> &str {
        "onedrive"
    }

    fn sync(&self, graph: &KnowledgeGraph, state: &SyncState) -> Result<SyncReport> {
        let mut report = SyncReport::default();
        let token = self.access_token()?;

        let start_url = state
            .get("onedrive", "delta_link")?
            .unwrap_or_else(|| {
                format!("{GRAPH_BASE}/root/children?$top=100")
            });

        let (items, new_delta) = self.fetch_pages(&token, &start_url)?;

        for item in items {
            // Skip deleted items and folders
            if item.deleted.is_some() || item.file.is_none() {
                report.skipped += 1;
                continue;
            }

            let node_id = format!("onedrive::{}", item.id);
            let description = item.description.unwrap_or_default();
            let url = item.web_url.unwrap_or_default();
            let node = Node {
                id: node_id,
                project_id: graph.project_id().to_string(),
                name: item.name.clone(),
                node_type: NodeType::Document,
                file_path: Some(url),
                start_line: None,
                end_line: None,
                summary: Some(description.chars().take(200).collect()),
                content_hash: None,
            };
            graph.add_node(&node)?;
            graph.index_fts(&node, &format!("{} {}", item.name, description))?;
            report.ingested += 1;
        }

        if let Some(link) = new_delta {
            state.set("onedrive", "delta_link", &link)?;
        }
        Ok(report)
    }
}
