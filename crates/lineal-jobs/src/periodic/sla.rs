//! Periodischer Job `sla` (WP12): SLA-Regeln und Meldungen (docs/03 §11). Die Logik liegt in
//! `App::run_sla`; je SLA und Schwelle wird genau einmal gemeldet (`IssueSlaSet`, `IssueSlaHighRisk`,
//! `IssueSlaBreached`).

use crate::{PeriodicJob, Schedule};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_app::{App, AppResult};
use std::time::Duration;

pub struct SlaJob;

#[async_trait]
impl PeriodicJob for SlaJob {
    fn name(&self) -> &'static str {
        "sla"
    }

    fn schedule(&self) -> Schedule {
        Schedule::Every(Duration::from_secs(60))
    }

    async fn run(&self, app: &App, now: DateTime<Utc>) -> AppResult<()> {
        let stats = app.run_sla(now).await?;
        if stats.processed > 0 {
            tracing::info!(processed = stats.processed, "sla updates");
        }
        if !stats.errors.is_empty() {
            tracing::warn!(errors = ?stats.errors, "sla job finished with errors");
        }
        Ok(())
    }
}
