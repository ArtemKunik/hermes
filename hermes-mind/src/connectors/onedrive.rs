// OneDrive connector via Microsoft Graph API + OAuth2.
//
// Required env vars:
//   HERMES_MIND_ONEDRIVE_CLIENT_ID
//   HERMES_MIND_ONEDRIVE_CLIENT_SECRET
//   HERMES_MIND_ONEDRIVE_REFRESH_TOKEN  — obtained via OAuth2 consent flow (one-time)
//
// Indexes file names, paths, and descriptions from the root drive.
// Full file content fetch is deferred to hermes mind_fetch.

use super::{Connector, SyncReport};
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
}

#[derive(Deserialize)]
struct DriveItem {
    id: String,
    name: String,
    #[serde(rename = "webUrl")]
    web_url: Option<String>,
    description: Option<String>,
    file: Option<serde_json::Value>,
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

    fn list_items(&self, token: &str) -> Result<Vec<DriveItem>> {
        let resp: DriveItemList =
            ureq::get(&format!("{GRAPH_BASE}/root/children"))
                .set("Authorization", &format!("Bearer {token}"))
                .query("$top", "100")
                .call()?
                .into_json()?;
        Ok(resp.value)
    }
}

impl Connector for OneDriveConnector {
    fn name(&self) -> &str {
        "onedrive"
    }

    fn sync(&self, graph: &KnowledgeGraph) -> Result<SyncReport> {
        let mut report = SyncReport::default();
        let token = self.access_token()?;
        let items = self.list_items(&token)?;

        for item in items {
            if item.file.is_none() {
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

        Ok(report)
    }
}
