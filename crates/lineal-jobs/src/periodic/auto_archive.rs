//! Periodischer Job `auto_archive` (WP12): archiviert lange geschlossene Issues (docs/03 §9.2).
//! Die Logik liegt in `App::run_auto_archive`; der Lauf ist idempotent.

use crate::{PeriodicJob, Schedule};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_app::{App, AppResult};
use std::time::Duration;

pub struct AutoArchiveJob;

#[async_trait]
impl PeriodicJob for AutoArchiveJob {
    fn name(&self) -> &'static str {
        "auto_archive"
    }

    fn schedule(&self) -> Schedule {
        Schedule::Every(Duration::from_secs(300))
    }

    async fn run(&self, app: &App, now: DateTime<Utc>) -> AppResult<()> {
        let stats = app.run_auto_archive(now).await?;
        if stats.processed > 0 {
            tracing::info!(processed = stats.processed, "auto-archived issues");
        }
        if !stats.errors.is_empty() {
            tracing::warn!(errors = ?stats.errors, "auto-archive finished with errors");
        }
        Ok(())
    }
}
