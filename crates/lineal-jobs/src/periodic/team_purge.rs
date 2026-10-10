//! Periodischer Job `team_purge` (WP16, docs/02 §2.7): löscht gelöschte Teams nach Ablauf von
//! `trashRetentionDays` endgültig. Die Logik liegt in `App::run_team_purge`.

use crate::{PeriodicJob, Schedule};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_app::{App, AppResult};
use std::time::Duration;

pub struct TeamPurgeJob;

#[async_trait]
impl PeriodicJob for TeamPurgeJob {
    fn name(&self) -> &'static str {
        "team_purge"
    }

    fn schedule(&self) -> Schedule {
        Schedule::Every(Duration::from_secs(3600))
    }

    async fn run(&self, app: &App, now: DateTime<Utc>) -> AppResult<()> {
        let stats = app.run_team_purge(now).await?;
        if stats.processed > 0 {
            tracing::info!(processed = stats.processed, "purged teams");
        }
        if !stats.errors.is_empty() {
            tracing::warn!(errors = ?stats.errors, "team purge finished with errors");
        }
        Ok(())
    }
}
