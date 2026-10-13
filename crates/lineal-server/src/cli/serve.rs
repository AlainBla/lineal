//! `lineal serve`.

use super::{connect, shutdown_grace, shutdown_signal};
use crate::build_router;
use anyhow::{Context, Result};
use sqlx::PgPool;

/// Hinweis für die Ersteinrichtung, wenn die Instanz noch keinen Workspace hat.
pub async fn warn_if_no_workspace(pool: &PgPool) {
    if matches!(has_workspace(pool).await, Ok(false)) {
        tracing::warn!(
            "no workspace exists yet, nobody can log in. Create the first workspace and owner: \
             `lineal create-workspace --name <name> --owner-email <email> --owner-name <name>` \
             (password from LINEAL_OWNER_PASSWORD or interactive prompt; with Docker: \
             `docker compose --profile app run --rm migrate create-workspace ...`)"
        );
    }
}

/// Gibt es mindestens einen Workspace?
pub async fn has_workspace(pool: &PgPool) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM organization)")
        .fetch_one(pool)
        .await
}

pub async fn run() -> Result<()> {
    let (config, pool, app) = connect().await?;
    warn_if_no_workspace(&pool).await;
    let router = build_router(app, config.dev);
    let listener = tokio::net::TcpListener::bind(config.bind)
        .await
        .with_context(|| format!("cannot bind {}", config.bind))?;
    tracing::info!(addr = %config.bind, dev = config.dev, "lineal listening");
    // `ConnectInfo` liefert dem Rate-Limit die Client-IP (unauthentifizierte Buckets).
    let stop = std::sync::Arc::new(tokio::sync::Notify::new());
    let stop_tx = stop.clone();
    let server = std::future::IntoFuture::into_future(
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            // WebSockets mit 1001 schliessen (Clients verbinden sich neu), bevor die Gnadenfrist laeuft.
            lineal_sync::begin_shutdown();
            stop_tx.notify_one();
        }),
    );
    tokio::pin!(server);
    let grace = shutdown_grace();
    tokio::select! {
        r = &mut server => r.context("server error"),
        () = async {
            stop.notified().await;
            tokio::time::sleep(grace).await;
        } => {
            // Offene WebSockets/Streams halten das geordnete Beenden sonst bis zum SIGKILL auf.
            tracing::warn!(grace_secs = grace.as_secs(), "open connections did not finish in time; exiting");
            Ok(())
        }
    }
}
