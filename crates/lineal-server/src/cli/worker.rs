//! `lineal worker`.

use super::{connect, shutdown_signal};
use anyhow::Result;
use lineal_jobs::{Worker, WorkerConfig};

pub async fn run() -> Result<()> {
    let (_config, _pool, app) = connect().await?;
    tracing::info!("worker started");
    Worker::new(app, WorkerConfig::default())
        .run(shutdown_signal())
        .await;
    Ok(())
}
