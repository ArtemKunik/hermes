// tools/hermes-engine/src/http_api/missions.rs
//
// HTTP handlers for /api/missions — list, get (with log), append event.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::http_api::error::{ApiError, ApiResult};
use crate::mission::{MissionStore, MissionStatus};
use crate::HermesEngine;

pub fn routes(engine: Arc<HermesEngine>) -> Router {
    Router::new()
        .route("/", get(list_missions))
        .route("/:id", get(get_mission))
        .route("/:id/events", post(append_event))
        .route("/:id/status", post(update_status))
        .with_state(engine)
}

#[derive(Deserialize, Default)]
struct ListQuery {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

async fn list_missions(
    State(engine): State<Arc<HermesEngine>>,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Value>> {
    let limit = q.limit.unwrap_or(50);
    let conn = engine.db().lock().map_err(|e| ApiError::internal(anyhow::anyhow!("db lock: {e}")))?;
    let store = MissionStore::new(&conn, engine.project_id());
    let items = store
        .list(q.status.as_deref(), limit)
        .map_err(ApiError::internal)?;
    Ok(Json(json!({
        "items": items,
        "total": items.len(),
        "limit": limit,
    })))
}

async fn get_mission(
    State(engine): State<Arc<HermesEngine>>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let conn = engine.db().lock().map_err(|e| ApiError::internal(anyhow::anyhow!("db lock: {e}")))?;
    let store = MissionStore::new(&conn, engine.project_id());
    let m = store.get(&id).map_err(ApiError::internal)?;
    let log = store.get_log(&id).map_err(ApiError::internal)?;
    Ok(Json(json!({
        "mission": m,
        "log": log,
    })))
}

#[derive(Deserialize)]
struct AppendEventBody {
    event_type: String,
    #[serde(default)]
    data: Option<Value>,
}

async fn append_event(
    State(engine): State<Arc<HermesEngine>>,
    Path(id): Path<String>,
    Json(body): Json<AppendEventBody>,
) -> ApiResult<impl IntoResponse> {
    if body.event_type.is_empty() {
        return Err(ApiError::bad_request("event_type is required"));
    }
    let conn = engine.db().lock().map_err(|e| ApiError::internal(anyhow::anyhow!("db lock: {e}")))?;
    let store = MissionStore::new(&conn, engine.project_id());
    // Touch the mission first to ensure it exists
    let _ = store.get(&id).map_err(ApiError::internal)?;
    store
        .append_log(&id, &body.event_type, body.data.as_ref().unwrap_or(&Value::Null))
        .map_err(ApiError::internal)?;
    Ok((StatusCode::CREATED, Json(json!({ "ok": true }))))
}

#[derive(Deserialize)]
struct UpdateStatusBody {
    status: String,
}

async fn update_status(
    State(engine): State<Arc<HermesEngine>>,
    Path(id): Path<String>,
    Json(body): Json<UpdateStatusBody>,
) -> ApiResult<Json<Value>> {
    if MissionStatus::parse(&body.status).is_none() {
        return Err(ApiError::bad_request(format!("invalid status '{}'", body.status)));
    }
    let conn = engine.db().lock().map_err(|e| ApiError::internal(anyhow::anyhow!("db lock: {e}")))?;
    let store = MissionStore::new(&conn, engine.project_id());
    let updated = store
        .update_status(&id, &body.status)
        .map_err(ApiError::internal)?;
    Ok(Json(serde_json::to_value(updated).unwrap_or_default()))
}
