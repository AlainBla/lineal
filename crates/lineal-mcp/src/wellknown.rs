//! `/.well-known/oauth-protected-resource` und `/.well-known/oauth-authorization-server` (WP04).
//! Im Scaffold 501.

/// Platzhalter-Handler.
pub async fn not_implemented() -> (axum::http::StatusCode, axum::Json<serde_json::Value>) {
    (
        axum::http::StatusCode::NOT_IMPLEMENTED,
        axum::Json(
            serde_json::json!({ "error": "internal", "message": "not implemented", "status": 501 }),
        ),
    )
}
