//! `lineal worker`.

use super::{connect, shutdown_signal};
use anyhow::Result;
use lineal_jobs::{Worker, WorkerConfig};
use std::time::Duration;

/// Heartbeat-Datei für den Container-Healthcheck (`docker-compose.yml`): wird alle 10 s
/// beschrieben; ein Healthcheck prüft ihr Alter. Pfad per `LINEAL_WORKER_HEARTBEAT`,
/// `off` schaltet ihn ab.
fn heartbeat_path() -> Option<String> {
    match std::env::var("LINEAL_WORKER_HEARTBEAT") {
        Ok(v) if v == "off" => None,
        Ok(v) if !v.is_empty() => Some(v),
        _ => Some("/tmp/lineal-worker.heartbeat".to_owned()),
    }
}

pub async fn run() -> Result<()> {
    let (_config, _pool, app) = connect().await?;
    tracing::info!("worker started");
    if let Some(path) = heartbeat_path() {
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(10));
            loop {
                tick.tick().await;
                if let Err(e) = tokio::fs::write(&path, chrono::Utc::now().to_rfc3339()).await {
                    tracing::debug!(error = %e, "cannot write the worker heartbeat");
                }
            }
        });
    }
    Worker::new(app, WorkerConfig::default())
        .run(shutdown_signal())
        .await;
    Ok(())
}
