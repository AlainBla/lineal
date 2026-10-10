//! Composition Root: verdrahtet Adapter und Ports zu `App`, Router und Worker.
#![forbid(unsafe_code)]

pub mod adapters;
pub mod cli;
pub mod config;

use axum::Router;
use lineal_app::App;
use std::sync::Arc;

/// Fügt die Router aller Adapter zusammen (api: graphql/auth/oauth/files/health, mcp, sync).
pub fn build_router(app: Arc<App>, dev: bool) -> Router {
    // Plattform-Middleware (Request-ID, Security-Header, Auth-Zustand, Rate-Limit, CORS im
    // Dev-Modus) einmal über ALLE Pfade legen, auch über /mcp und /sync.
    let routes = lineal_api::routes(app.clone(), dev)
        .merge(lineal_mcp::router(app.clone()))
        .merge(lineal_sync::router(app.clone()));
    lineal_api::middleware::apply(routes, app, dev)
}
