// tools/hermes-engine/src/http_api/mod.rs
//
// Lightweight HTTP API for the Hermes engine. Exposes proposal and mission
// CRUD so external services (e.g. the mastermind daemon) can write to the
// same System 2 store without going through MCP.

pub mod error;
pub mod missions;
pub mod proposals;

use axum::{routing::get, Json, Router};
use serde::Serialize;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

use crate::accounting::{Accountant, SlowToolCall, ToolCallStat};
use crate::HermesEngine;

pub fn build_router(engine: Arc<HermesEngine>) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .nest("/api/proposals", proposals::routes(engine.clone()))
        .nest("/api/missions", missions::routes(engine))
        .route("/api/observability/tool-stats", get(tool_stats_handler))
        .route("/api/observability/slow-tools", get(slow_tools_handler))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
}

async fn tool_stats_handler(
    engine: axum::extract::State<Arc<HermesEngine>>,
) -> Json<serde_json::Value> {
    let acct = Accountant::new(engine.db().clone(), engine.project_id(), engine.session_id());
    match acct.get_tool_call_stats() {
        Ok(stats) => Json(serde_json::json!({ "tool_stats": stats })),
        Err(e) => Json(serde_json::json!({ "error": e.to_string() })),
    }
}

async fn slow_tools_handler(
    engine: axum::extract::State<Arc<HermesEngine>>,
) -> Json<serde_json::Value> {
    let threshold_ms = std::env::var("HERMES_OBSERVABILITY_SLOW_THRESHOLD_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(5000);

    let acct = Accountant::new(engine.db().clone(), engine.project_id(), engine.session_id());
    match acct.get_slow_tool_calls(threshold_ms) {
        Ok(slow) => Json(serde_json::json!({ "slow_tools": slow, "threshold_ms": threshold_ms })),
        Err(e) => Json(serde_json::json!({ "error": e.to_string() })),
    }
}

/// Resolve the HTTP port from the `HERMES_HTTP_PORT` env var (default 38081).
pub fn resolve_http_port() -> u16 {
    std::env::var("HERMES_HTTP_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(38081)
}
