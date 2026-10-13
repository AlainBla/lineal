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
        purge_queues(app, now).await?;
        Ok(())
    }
}

/// Aufbewahrung erledigter Jobs (1 Tag), fehlgeschlagener Jobs (14 Tage) und verarbeiteter
/// Outbox-Events (7 Tage; mit Dead-Letter 30 Tage). In Blöcken, damit kein Löschlauf lange sperrt.
pub const JOB_DONE_RETENTION_DAYS: i64 = 1;
pub const JOB_FAILED_RETENTION_DAYS: i64 = 14;
pub const OUTBOX_RETENTION_DAYS: i64 = 7;
pub const OUTBOX_DEAD_LETTER_RETENTION_DAYS: i64 = 30;
const PURGE_BLOCK: i64 = 5_000;
const PURGE_MAX_BLOCKS: usize = 20;

pub async fn purge_queues(app: &App, now: DateTime<Utc>) -> AppResult<(u64, u64)> {
    let days = chrono::Duration::days;
    let (mut jobs, mut events) = (0u64, 0u64);
    for _ in 0..PURGE_MAX_BLOCKS {
        let mut tx = app.store.begin_system().await?;
        let j = tx
            .jobs()
            .purge_finished(
                now - days(JOB_DONE_RETENTION_DAYS),
                now - days(JOB_FAILED_RETENTION_DAYS),
                PURGE_BLOCK,
            )
            .await?;
        let e = tx
            .outbox()
            .purge_processed(
                now - days(OUTBOX_RETENTION_DAYS),
                now - days(OUTBOX_DEAD_LETTER_RETENTION_DAYS),
                PURGE_BLOCK,
            )
            .await?;
        tx.commit().await?;
        jobs += j;
        events += e;
        if j < PURGE_BLOCK as u64 && e < PURGE_BLOCK as u64 {
            break;
        }
    }
    if jobs > 0 || events > 0 {
        tracing::info!(
            jobs,
            events,
            "purged finished jobs and processed outbox events"
        );
    }
    Ok((jobs, events))
}
