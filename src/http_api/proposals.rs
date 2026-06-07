// tools/hermes-engine/src/http_api/proposals.rs
//
// HTTP handlers for /api/proposals — create, list, get, update status.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, patch, post},
    Json, Router,
};
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::http_api::error::{ApiError, ApiResult};
use crate::proposal::ProposalStore;
use crate::HermesEngine;

pub fn routes(engine: Arc<HermesEngine>) -> Router {
    Router::new()
        .route("/", post(create_proposals).get(list_proposals))
        .route("/{id}", get(get_proposal))
        .route("/{id}/status", patch(update_proposal_status))
        .with_state(engine)
}

#[derive(Deserialize, Default)]
struct ListQuery {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

async fn create_proposals(
    State(engine): State<Arc<HermesEngine>>,
    Json(body): Json<Value>,
) -> ApiResult<impl IntoResponse> {
    let proposals = body["proposals"]
        .as_array()
        .ok_or_else(|| ApiError::bad_request("body requires 'proposals' array"))?;

    let conn = engine
        .db()
        .lock()
        .map_err(|e| ApiError::internal(anyhow::anyhow!("db lock: {e}")))?;
    let store = ProposalStore::new(&conn, engine.project_id());
    let created = store.create_batch(proposals).map_err(ApiError::internal)?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "created": created.len(),
            "proposals": created,
        })),
    ))
}

async fn list_proposals(
    State(engine): State<Arc<HermesEngine>>,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Value>> {
    let limit = q.limit.unwrap_or(50);
    let conn = engine
        .db()
        .lock()
        .map_err(|e| ApiError::internal(anyhow::anyhow!("db lock: {e}")))?;
    let store = ProposalStore::new(&conn, engine.project_id());
    let items = store
        .list(q.status.as_deref(), q.source.as_deref(), limit)
        .map_err(ApiError::internal)?;
    Ok(Json(json!({
        "items": items,
        "total": items.len(),
        "limit": limit,
    })))
}

async fn get_proposal(
    State(engine): State<Arc<HermesEngine>>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let conn = engine
        .db()
        .lock()
        .map_err(|e| ApiError::internal(anyhow::anyhow!("db lock: {e}")))?;
    let store = ProposalStore::new(&conn, engine.project_id());
    let p = store.get(&id)?;
    Ok(Json(serde_json::to_value(p).unwrap_or_default()))
}

#[derive(Deserialize)]
struct UpdateStatusBody {
    status: String,
}

async fn update_proposal_status(
    State(engine): State<Arc<HermesEngine>>,
    Path(id): Path<String>,
    Json(body): Json<UpdateStatusBody>,
) -> ApiResult<Json<Value>> {
    if !is_valid_proposal_status(&body.status) {
        return Err(ApiError::bad_request(format!(
            "invalid status '{}', expected one of: pending, edited, approved, rejected, completed",
            body.status
        )));
    }

    let conn = engine
        .db()
        .lock()
        .map_err(|e| ApiError::internal(anyhow::anyhow!("db lock: {e}")))?;
    let store = ProposalStore::new(&conn, engine.project_id());
    let updated = store.update_status(&id, &body.status)?;
    Ok(Json(serde_json::to_value(updated).unwrap_or_default()))
}

fn is_valid_proposal_status(s: &str) -> bool {
    matches!(
        s,
        "pending" | "edited" | "approved" | "rejected" | "completed"
    )
}

/// Helper for tests / future expansion: get a Connection from an engine.
pub fn conn(engine: &HermesEngine) -> std::sync::MutexGuard<'_, Connection> {
    engine.db().lock().expect("db lock")
}
