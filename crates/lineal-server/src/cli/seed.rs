//! `lineal seed`: legt den Beispiel-Workspace `acme` an (Seed-Module aus `lineal-testkit`).

use anyhow::{Result, bail};

/// `seed` legt Benutzer mit dem bekannten Passwort `lineal-dev` an und ist daher nur im
/// Dev-Modus (`LINEAL_DEV=1`) oder mit ausdrücklichem `--allow-insecure-demo-users` erlaubt.
pub fn check_allowed(dev: bool, allow_insecure: bool) -> Result<()> {
    if dev {
        return Ok(());
    }
    if allow_insecure {
        tracing::warn!(
            "seeding demo users with the publicly known password `lineal-dev` \
             (--allow-insecure-demo-users); never do this on a reachable instance"
        );
        return Ok(());
    }
    bail!(
        "`lineal seed` creates users with a publicly known password. It only runs with \
         LINEAL_DEV=1 or --allow-insecure-demo-users. For a real instance use \
         `lineal create-workspace` instead."
    )
}

#[cfg(feature = "seed")]
pub async fn run(demo: bool, allow_insecure: bool) -> Result<()> {
    check_allowed(crate::config::dev_from_env(), allow_insecure)?;
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
pub async fn run(_demo: bool, _allow_insecure: bool) -> Result<()> {
    anyhow::bail!("built without the `seed` feature")
}

#[cfg(test)]
mod tests {
    use super::check_allowed;

    #[test]
    fn seed_requires_dev_or_explicit_flag() {
        assert!(check_allowed(false, false).is_err());
        assert!(check_allowed(true, false).is_ok());
        assert!(check_allowed(false, true).is_ok());
    }
}
