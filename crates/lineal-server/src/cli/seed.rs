//! `lineal seed`: legt den Beispiel-Workspace `acme` an (Seed-Module aus `lineal-testkit`).

use anyhow::Result;

#[cfg(feature = "seed")]
pub async fn run(demo: bool) -> Result<()> {
    let (_config, _pool, app) = super::connect().await?;
    let org = lineal_testkit::seed::SeedData::acme().org;
    // Die Repos arbeiten im Org-Kontext; mit begin_system() bekämen sie die nil-Org.
    let mut tx = app
        .store
        .begin(org)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    // Mit `--demo` darf der Basis-Workspace schon existieren (Demo-Daten kommen obendrauf).
    let exists = demo
        && tx
            .orgs()
            .get()
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?
            .is_some();
    if !exists {
        let data = lineal_testkit::seed::run(&mut *tx)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        tracing::info!(org = %data.org, "seed finished");
    }
    tx.commit().await.map_err(|e| anyhow::anyhow!("{e}"))?;
    if demo {
        lineal_testkit::seed::demo::run(&app)
            .await
            .map_err(|e| anyhow::anyhow!("demo seed failed: {e}"))?;
        tracing::info!("demo seed finished");
    }
    Ok(())
}

#[cfg(not(feature = "seed"))]
pub async fn run(_demo: bool) -> Result<()> {
    anyhow::bail!("built without the `seed` feature")
}
