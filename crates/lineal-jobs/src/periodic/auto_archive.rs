//! Periodischer Job `auto_archive` (WP12): archiviert lange geschlossene Issues (docs/03 §9.2).
//! Die Logik liegt in `App::run_auto_archive` (Issues), `run_planning_auto_archive` (Cycles,
//! Projekte) und `run_auto_archive_warnings` (Vorwarnung); die Läufe sind idempotent.

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
        // Cycles und Projekte (docs/04), danach die Vorwarnung für bald archivierte Issues.
        let planning = app.run_planning_auto_archive(now).await?;
        if planning.processed > 0 {
            tracing::info!(
                processed = planning.processed,
                "auto-archived cycles and projects"
            );
        }
        if !planning.errors.is_empty() {
            tracing::warn!(errors = ?planning.errors, "planning auto-archive finished with errors");
        }
        let warned = app.run_auto_archive_warnings(now).await?;
        if !warned.errors.is_empty() {
            tracing::warn!(errors = ?warned.errors, "auto-archive warnings finished with errors");
        }
        Ok(())
    }
}
