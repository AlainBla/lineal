//! Periodischer Job `triage_rotation` (WP12): Schichtwechsel der Triage-Rotation und
//! Snooze-Wiedervorlage in der Triage (docs/10 §3, docs/03 §10.3). Die Logik liegt in
//! `App::run_triage_rotation` und `App::run_triage_unsnooze`; beide speichern nur bei Änderungen.

use crate::{PeriodicJob, Schedule};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_app::{App, AppResult};
use std::time::Duration;

pub struct TriageRotationJob;

#[async_trait]
impl PeriodicJob for TriageRotationJob {
    fn name(&self) -> &'static str {
        "triage_rotation"
    }

    fn schedule(&self) -> Schedule {
        Schedule::Every(Duration::from_secs(60))
    }

    async fn run(&self, app: &App, now: DateTime<Utc>) -> AppResult<()> {
        let rotation = app.run_triage_rotation(now).await?;
        let snooze = app.run_triage_unsnooze(now).await?;
        if rotation.processed + snooze.processed > 0 {
            tracing::info!(
                rotations = rotation.processed,
                unsnoozed = snooze.processed,
                "triage rotation"
            );
        }
        for e in rotation.errors.iter().chain(snooze.errors.iter()) {
            tracing::warn!(error = %e, "triage rotation unit failed");
        }
        Ok(())
    }
}
