//! Periodischer Job `purge` (WP12): löscht Issues nach Ablauf der Papierkorb-Frist endgültig
//! (docs/03 §9.1). Die Logik liegt in `App::run_purge`; das Löschen eines fehlenden Datensatzes
//! ist ein No-op.

use crate::{PeriodicJob, Schedule};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_app::{App, AppResult};
use std::time::Duration;

pub struct PurgeJob;

#[async_trait]
impl PeriodicJob for PurgeJob {
    fn name(&self) -> &'static str {
        "purge"
    }

    fn schedule(&self) -> Schedule {
        Schedule::Every(Duration::from_secs(300))
    }

    async fn run(&self, app: &App, now: DateTime<Utc>) -> AppResult<()> {
        let stats = app.run_purge(now).await?;
        if stats.processed > 0 {
            tracing::info!(processed = stats.processed, "purged issues");
        }
        if !stats.errors.is_empty() {
            tracing::warn!(errors = ?stats.errors, "purge finished with errors");
        }
        Ok(())
    }
}
