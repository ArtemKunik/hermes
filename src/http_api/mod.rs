// tools/hermes-engine/src/http_api/mod.rs
//
// Lightweight HTTP API for the Hermes engine. Exposes proposal and mission
// CRUD so external services (e.g. the mastermind daemon) can write to the
// same System 2 store without going through MCP.

pub mod error;
pub mod missions;
pub mod proposals;

use axum::Router;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

use crate::HermesEngine;

pub fn build_router(engine: Arc<HermesEngine>) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .nest("/api/proposals", proposals::routes(engine.clone()))
        .nest("/api/missions", missions::routes(engine))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
}

/// Resolve the HTTP port from the `HERMES_HTTP_PORT` env var (default 38081).
pub fn resolve_http_port() -> u16 {
    std::env::var("HERMES_HTTP_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(38081)
}
