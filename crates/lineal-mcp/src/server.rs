//! MCP-Router (WP04 ersetzt die 501-Platzhalter durch Streamable HTTP).

use crate::wellknown::not_implemented;
use axum::Router;
use axum::routing::{get, post};
use lineal_app::App;
use std::sync::Arc;

/// Routen des MCP-Adapters: `/mcp` und die OAuth-Discovery-Metadaten.
pub fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/mcp", post(not_implemented).get(not_implemented))
        .route(
            "/.well-known/oauth-protected-resource",
            get(not_implemented),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            get(not_implemented),
        )
        .with_state(app)
}
