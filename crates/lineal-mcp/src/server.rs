//! MCP-Router: `/mcp` (Streamable HTTP) und die OAuth-Discovery-Metadaten.

use crate::transport::{delete_mcp, get_mcp, post_mcp};
use crate::wellknown::{authorization_server, protected_resource};
use axum::Router;
use axum::routing::{get, post};
use lineal_app::App;
use std::sync::Arc;

/// Routen des MCP-Adapters: `/mcp` und die OAuth-Discovery-Metadaten
/// (RFC 9728 auch mit Pfadsuffix `/mcp`).
pub fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/mcp", post(post_mcp).get(get_mcp).delete(delete_mcp))
        .route(
            "/.well-known/oauth-protected-resource",
            get(protected_resource),
        )
        .route(
            "/.well-known/oauth-protected-resource/mcp",
            get(protected_resource),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            get(authorization_server),
        )
        .with_state(app)
}
