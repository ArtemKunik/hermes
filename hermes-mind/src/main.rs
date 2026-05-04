mod auth;
mod connectors;
mod mcp_server;
mod schema;
mod sync_state;

use anyhow::Result;
use clap::{Parser, Subcommand};
use connectors::{
    gmail::GmailConnector, onedrive::OneDriveConnector, telegram::TelegramConnector,
    whatsapp::WhatsAppConnector, Connector,
};
use hermes_engine::{graph::KnowledgeGraph, HermesEngine};
use sync_state::SyncState;
use std::{env, path::PathBuf};

#[derive(Parser)]
#[command(
    name = "hermes-mind",
    about = "Personal knowledge connector for hermes",
    after_help = "\
Environment variables:
  HERMES_PROJECT_ROOT                  Project root (default: cwd)
  HERMES_MIND_DB_PATH                  SQLite DB path (default: <root>/.hermes-mind.db)
  HERMES_MIND_WA_SIDECAR_PATH          Path to Baileys Node.js sidecar (index.js)
  HERMES_MIND_WA_CREDS_PATH            Directory for WhatsApp creds.json
  HERMES_MIND_GMAIL_CLIENT_ID
  HERMES_MIND_GMAIL_CLIENT_SECRET
  HERMES_MIND_GMAIL_REFRESH_TOKEN
  HERMES_MIND_ONEDRIVE_CLIENT_ID
  HERMES_MIND_ONEDRIVE_CLIENT_SECRET
  HERMES_MIND_ONEDRIVE_REFRESH_TOKEN
  HERMES_MIND_TELEGRAM_BOT_TOKEN"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Run as MCP JSON-RPC 2.0 stdio server
    #[arg(long)]
    stdio: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Sync all or a specific connector
    Sync {
        /// Connector name: whatsapp, gmail, onedrive, telegram
        #[arg(long)]
        connector: Option<String>,
    },
    /// List active connectors
    Status,
    /// Run one-time OAuth2 consent flow for a connector
    Auth {
        /// Connector to authenticate: gmail, onedrive
        connector: String,
        #[arg(long)]
        client_id: String,
        #[arg(long)]
        client_secret: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let (engine, project_root) = open_engine()?;
    let connectors = build_connectors();

    if cli.stdio {
        return mcp_server::run(&engine, &project_root, connectors);
    }

    match cli.command.unwrap() {
        Commands::Sync { connector } => cmd_sync(&engine, &project_root, &connectors, connector.as_deref()),
        Commands::Status => cmd_status(&connectors),
        Commands::Auth { connector, client_id, client_secret } => {
            match connector.as_str() {
                "gmail"    => auth::auth_gmail(&client_id, &client_secret),
                "onedrive" => auth::auth_onedrive(&client_id, &client_secret),
                other => anyhow::bail!("unknown connector '{other}' — supported: gmail, onedrive"),
            }
        }
    }
}

fn open_engine() -> Result<(HermesEngine, PathBuf)> {
    let project_root = env::var("HERMES_PROJECT_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let db_path = env::var("HERMES_MIND_DB_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| project_root.join(".hermes-mind.db"));

    let project_id = project_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    let engine = HermesEngine::new(&db_path, &project_id)?;
    schema::run_migrations(engine.db())?;
    Ok((engine, project_root))
}

fn build_connectors() -> Vec<Box<dyn Connector>> {
    let mut out: Vec<Box<dyn Connector>> = Vec::new();

    if let (Ok(sidecar), Ok(creds)) = (
        env::var("HERMES_MIND_WA_SIDECAR_PATH"),
        env::var("HERMES_MIND_WA_CREDS_PATH"),
    ) {
        out.push(Box::new(WhatsAppConnector::new(sidecar, creds)));
    }

    if let Ok(token) = env::var("HERMES_MIND_GMAIL_REFRESH_TOKEN") {
        out.push(Box::new(GmailConnector::new(
            env::var("HERMES_MIND_GMAIL_CLIENT_ID").unwrap_or_default(),
            env::var("HERMES_MIND_GMAIL_CLIENT_SECRET").unwrap_or_default(),
            token,
        )));
    }

    if let Ok(token) = env::var("HERMES_MIND_ONEDRIVE_REFRESH_TOKEN") {
        out.push(Box::new(OneDriveConnector::new(
            env::var("HERMES_MIND_ONEDRIVE_CLIENT_ID").unwrap_or_default(),
            env::var("HERMES_MIND_ONEDRIVE_CLIENT_SECRET").unwrap_or_default(),
            token,
        )));
    }

    if let Ok(token) = env::var("HERMES_MIND_TELEGRAM_BOT_TOKEN") {
        out.push(Box::new(TelegramConnector::new(token)));
    }

    out
}

fn cmd_sync(
    engine: &HermesEngine,
    project_root: &std::path::Path,
    connectors: &[Box<dyn Connector>],
    filter: Option<&str>,
) -> Result<()> {
    if connectors.is_empty() {
        eprintln!("[hermes-mind] no connectors configured — set env vars and retry");
        return Ok(());
    }
    let graph = KnowledgeGraph::new(engine.db().clone(), engine.project_id());
    let sync_state = SyncState::new(engine.db().clone(), engine.project_id());
    for connector in connectors {
        if let Some(name) = filter {
            if connector.name() != name {
                continue;
            }
        }
        eprintln!("[hermes-mind] syncing {}...", connector.name());
        match connector.sync(&graph, &sync_state) {
            Ok(r) => eprintln!(
                "[hermes-mind] {}: {} ingested, {} skipped, {} errors",
                connector.name(),
                r.ingested,
                r.skipped,
                r.errors
            ),
            Err(e) => eprintln!("[hermes-mind] {} failed: {e}", connector.name()),
        }
    }
    engine.invalidate_search_cache();
    let _ = project_root;
    Ok(())
}

fn cmd_status(connectors: &[Box<dyn Connector>]) -> Result<()> {
    if connectors.is_empty() {
        println!("No connectors configured.");
    } else {
        println!("Active connectors:");
        for c in connectors {
            println!("  • {}", c.name());
        }
    }
    Ok(())
}
