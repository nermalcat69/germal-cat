use crate::recorder;
use crate::state::Daemon;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use germalcat_core::SessionMeta;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tower_http::cors::CorsLayer;

pub fn router(daemon: Arc<Daemon>) -> Router {
    Router::new()
        .route("/api/status", get(status))
        .route("/api/unlock", post(unlock))
        .route("/api/reset", post(reset))
        .route("/api/record", post(record))
        .route("/api/sessions", get(list))
        .route("/api/sessions/:id", get(meta))
        .route("/api/sessions/:id/har", get(har))
        .route("/api/sessions/:id/console", get(console))
        .route("/api/sessions/:id/stop", post(stop))
        .layer(CorsLayer::permissive())
        .with_state(daemon)
}

type ApiErr = (StatusCode, Json<Value>);

fn err(code: StatusCode, e: impl std::fmt::Display) -> ApiErr {
    (code, Json(json!({ "error": e.to_string() })))
}

#[derive(Deserialize)]
struct Password {
    password: String,
}

#[derive(Deserialize)]
struct RecordReq {
    url: String,
    #[serde(default = "default_browser")]
    browser: String,
}
fn default_browser() -> String {
    "chrome".into()
}

async fn status(State(d): State<Arc<Daemon>>) -> Json<Value> {
    Json(json!({
        "initialized": d.vault.is_initialized(),
        "unlocked": d.is_unlocked(),
        "active": d.active_ids(),
    }))
}

async fn unlock(
    State(d): State<Arc<Daemon>>,
    Json(p): Json<Password>,
) -> Result<Json<Value>, ApiErr> {
    d.unlock(&p.password).map_err(|e| err(StatusCode::UNAUTHORIZED, e))?;
    Ok(Json(json!({ "ok": true })))
}

async fn reset(
    State(d): State<Arc<Daemon>>,
    Json(p): Json<Password>,
) -> Result<Json<Value>, ApiErr> {
    d.reset(&p.password).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(json!({ "ok": true, "wiped": true })))
}

async fn record(
    State(d): State<Arc<Daemon>>,
    Json(req): Json<RecordReq>,
) -> Result<Json<SessionMeta>, ApiErr> {
    let store = d.store().map_err(|e| err(StatusCode::UNAUTHORIZED, e))?;
    let meta = SessionMeta::new(&req.url, &req.browser);
    store.save_meta(&meta).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let daemon = d.clone();
    let m = meta.clone();
    tokio::spawn(async move {
        if let Err(e) = recorder::run(daemon, m).await {
            tracing::error!("recording failed: {e:#}");
        }
    });
    Ok(Json(meta))
}

async fn list(State(d): State<Arc<Daemon>>) -> Result<Json<Vec<SessionMeta>>, ApiErr> {
    Ok(Json(d.store().map_err(|e| err(StatusCode::UNAUTHORIZED, e))?.list()))
}

async fn meta(
    State(d): State<Arc<Daemon>>,
    Path(id): Path<String>,
) -> Result<Json<SessionMeta>, ApiErr> {
    let store = d.store().map_err(|e| err(StatusCode::UNAUTHORIZED, e))?;
    store.load_meta(&id).map(Json).map_err(|e| err(StatusCode::NOT_FOUND, e))
}

async fn har(
    State(d): State<Arc<Daemon>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiErr> {
    let store = d.store().map_err(|e| err(StatusCode::UNAUTHORIZED, e))?;
    let bytes = store.load_har(&id).map_err(|e| err(StatusCode::NOT_FOUND, e))?;
    Ok(([("content-type", "application/json")], bytes))
}

async fn console(
    State(d): State<Arc<Daemon>>,
    Path(id): Path<String>,
) -> Result<String, ApiErr> {
    let store = d.store().map_err(|e| err(StatusCode::UNAUTHORIZED, e))?;
    let bytes = store.load_console(&id).map_err(|e| err(StatusCode::NOT_FOUND, e))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

async fn stop(
    State(d): State<Arc<Daemon>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiErr> {
    if d.stop(&id) {
        Ok(Json(json!({ "ok": true })))
    } else {
        Err(err(StatusCode::NOT_FOUND, "no active recording with that id"))
    }
}
