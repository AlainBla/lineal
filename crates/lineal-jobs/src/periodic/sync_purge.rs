//! Periodischer Job `sync_purge`: entfernt Sync-Aktionen älter als 14 Tage (docs/09 §5).
//! Clients mit älterem `since` erhalten danach `resetRequired`.

use crate::{PeriodicJob, Schedule};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_app::{App, AppResult};
use std::time::Duration;

/// Aufbewahrung des Sync-Logs.
pub const SYNC_LOG_RETENTION_DAYS: i64 = 14;

pub struct SyncPurgeJob;

#[async_trait]
impl PeriodicJob for SyncPurgeJob {
    fn name(&self) -> &'static str {
        "sync_purge"
    }

    fn schedule(&self) -> Schedule {
        Schedule::Every(Duration::from_secs(3600))
    }

    async fn run(&self, app: &App, now: DateTime<Utc>) -> AppResult<()> {
        let before = now - chrono::Duration::days(SYNC_LOG_RETENTION_DAYS);
        let mut tx = app.store.begin_system().await?;
        let n = tx.outbox().sync_purge(before).await?;
        tx.commit().await?;
        if n > 0 {
            tracing::info!(actions = n, "sync log purged");
        }
        Ok(())
    }
}
