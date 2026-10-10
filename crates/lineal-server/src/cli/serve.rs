//! `lineal serve`.

use super::{connect, shutdown_signal};
use crate::build_router;
use anyhow::{Context, Result};

pub async fn run() -> Result<()> {
    let (config, _pool, app) = connect().await?;
    let router = build_router(app, config.dev);
    let listener = tokio::net::TcpListener::bind(config.bind)
        .await
        .with_context(|| format!("cannot bind {}", config.bind))?;
    tracing::info!(addr = %config.bind, dev = config.dev, "lineal listening");
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("server error")
}
