//! `lineal migrate`.

use super::connect;
use anyhow::{Context, Result};

pub async fn run() -> Result<()> {
    let (_config, pool, _app) = connect(false).await?;
    lineal_db::migrate(&pool)
        .await
        .context("migration failed")?;
    tracing::info!("migrations applied");
    Ok(())
}
