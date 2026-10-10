//! `lineal seed`: legt den Beispiel-Workspace `acme` an (Seed-Module aus `lineal-testkit`).

use anyhow::Result;

#[cfg(feature = "seed")]
pub async fn run() -> Result<()> {
    let (_config, _pool, app) = super::connect().await?;
    // Die Repos arbeiten im Org-Kontext; mit begin_system() bekämen sie die nil-Org.
    let mut tx = app
        .store
        .begin(lineal_testkit::seed::SeedData::acme().org)
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
