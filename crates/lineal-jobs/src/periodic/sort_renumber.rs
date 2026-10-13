//! Periodischer Job `sort_renumber` (B7.10): nummeriert die manuelle Sortierung (`sortOrder`)
//! eines Teams neu, wenn der Abstand zweier Nachbarn unter 1e-6 fällt (docs/03 §12). Die Logik
//! liegt in `App::run_sort_renumber`.

use crate::{PeriodicJob, Schedule};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_app::{App, AppResult};
use std::time::Duration;

pub struct SortRenumberJob;

#[async_trait]
impl PeriodicJob for SortRenumberJob {
    fn name(&self) -> &'static str {
        "sort_renumber"
    }

    fn schedule(&self) -> Schedule {
        Schedule::Every(Duration::from_secs(120))
    }

    async fn run(&self, app: &App, now: DateTime<Utc>) -> AppResult<()> {
        let stats = app.run_sort_renumber(now).await?;
        if stats.processed > 0 {
            tracing::info!(issues = stats.processed, "manual sort order renumbered");
        }
        if !stats.errors.is_empty() {
            tracing::warn!(errors = ?stats.errors, "sort renumbering finished with errors");
        }
        Ok(())
    }
}
