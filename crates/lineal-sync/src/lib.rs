//! Sync-Adapter: WebSocket-Delta-Stream `GET /sync` (WP15). Im Scaffold 501.
#![forbid(unsafe_code)]

use axum::routing::get;
use axum::{Json, Router, http::StatusCode};
use lineal_app::App;
use serde_json::{Value, json};
use std::sync::Arc;

async fn not_implemented() -> (StatusCode, Json<Value>) {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({ "error": "internal", "message": "not implemented", "status": 501 })),
    )
}

/// Routen des Sync-Adapters.
pub fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/sync", get(not_implemented))
        .with_state(app)
}
