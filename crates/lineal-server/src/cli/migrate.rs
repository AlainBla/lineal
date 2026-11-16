//! `lineal migrate`.

use super::connect;
use anyhow::{Context, Result};

pub async fn run() -> Result<()> {
    // Migrationen laufen als Eigentuemer (`DATABASE_URL`), nicht als `lineal_app`.
    let (config, pool, _app) = connect(false, false).await?;
    lineal_db::migrate(&pool)
        .await
        .context("migration failed")?;
    tracing::info!("migrations applied");

    // LIN-61/S-09: de-privilegierte Rolle `lineal_app` anlegen/aktualisieren und berechtigen, damit
    // Server und Worker sich damit verbinden koennen. Ohne Passwort uebersprungen (Dev/Test,
    // bestehende Installationen bleiben unveraendert und verbinden weiter als Eigentuemer).
    match config.app_db_password.as_deref() {
        Some(password) => {
            lineal_db::ensure_app_role(&pool, password).await.context(
                "cannot provision the de-privileged app role `lineal_app` (LIN-61/S-09)",
            )?;
            tracing::info!(
                role = lineal_db::APP_ROLE,
                "de-privileged app role provisioned (DML grants applied)"
            );
        }
        None => tracing::debug!(
            "LINEAL_APP_DB_PASSWORD not set; skipping app-role provisioning (server/worker connect as owner)"
        ),
    }
    Ok(())
}
