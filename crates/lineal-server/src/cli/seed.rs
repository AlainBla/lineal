//! `lineal seed`: legt den Beispiel-Workspace `acme` an (Seed-Module aus `lineal-testkit`).

use anyhow::Result;

#[cfg(feature = "seed")]
pub async fn run() -> Result<()> {
    let (_config, _pool, app) = super::connect().await?;
    let mut tx = app
        .store
        .begin_system()
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let data = lineal_testkit::seed::run(&mut *tx)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    tx.commit().await.map_err(|e| anyhow::anyhow!("{e}"))?;
    tracing::info!(org = %data.org, "seed finished");
    Ok(())
}

#[cfg(not(feature = "seed"))]
pub async fn run() -> Result<()> {
    anyhow::bail!("built without the `seed` feature")
}
