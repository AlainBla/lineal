//! REST-Endpunkte. Gemeinsame Datei: nur S1 / Orchestrator.

pub mod auth;

use axum::Json;
use axum::http::StatusCode;
use serde_json::{Value, json};

/// Antwort aller noch nicht implementierten Endpunkte (Fehlerform nach 00 §7).
pub async fn not_implemented() -> (StatusCode, Json<Value>) {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({ "error": "internal", "message": "not implemented", "status": 501 })),
    )
}
