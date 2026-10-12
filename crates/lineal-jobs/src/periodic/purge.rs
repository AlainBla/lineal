//! Periodischer Job `purge` (WP12): löscht Issues nach Ablauf der Papierkorb-Frist endgültig
//! (docs/03 §9.1), außerdem Dokumente und Projekte (`App::run_trash_purge`). Die Logik liegt in
//! `App::run_purge`; das Löschen eines fehlenden Datensatzes
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
        // Dokumente und Projekte im Papierkorb sowie verwaiste Dokumentinhalte (docs/06 §1.10).
        let trash = app.run_trash_purge(now).await?;
        if trash.processed > 0 {
            tracing::info!(processed = trash.processed, "purged documents and projects");
        }
        if !trash.errors.is_empty() {
            tracing::warn!(errors = ?trash.errors, "trash purge finished with errors");
        }
        let reservations = app.run_upload_purge(now).await?;
        if reservations > 0 {
            tracing::info!(reservations, "purged expired upload reservations");
        }
        Ok(())
    }
}
