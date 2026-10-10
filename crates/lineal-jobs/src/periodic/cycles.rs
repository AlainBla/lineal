//! Periodischer Job `cycles` (WP07, docs/04 §1.5): Cycle-Wechsel (Abschluss, Start, Rollover),
//! Generator-Reparaturlauf, Zeitplan-Neuberechnung, Tages-Snapshots und Projekt-Wochen-Snapshots.
//!
//! Die Logik liegt in `lineal_app::usecases::cycle` (`App::run_cycle_maintenance`); der Job ruft sie
//! minütlich auf. Der Lauf ist idempotent und pro Team durch eine Team-Sperre geschützt.

use crate::{PeriodicJob, Schedule};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_app::{App, AppResult};
use std::time::Duration;

pub struct CyclesJob;

#[async_trait]
impl PeriodicJob for CyclesJob {
    fn name(&self) -> &'static str {
        "cycles"
    }

    fn schedule(&self) -> Schedule {
        Schedule::Every(Duration::from_secs(60))
    }

    async fn run(&self, app: &App, now: DateTime<Utc>) -> AppResult<()> {
        let stats = app.run_cycle_maintenance(now).await?;
        if !stats.errors.is_empty() {
            tracing::warn!(errors = ?stats.errors, "cycle job finished with errors");
        }
        Ok(())
    }
}
