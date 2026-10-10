//! Periodischer Job `auto_close` (WP12): schließt offene Issues ohne Aktualisierung (docs/03 §9.3).
//! Die Logik liegt in `App::run_auto_close`; der Lauf ist idempotent.

use crate::{PeriodicJob, Schedule};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_app::{App, AppResult};
use std::time::Duration;

pub struct AutoCloseJob;

#[async_trait]
impl PeriodicJob for AutoCloseJob {
    fn name(&self) -> &'static str {
        "auto_close"
    }

    fn schedule(&self) -> Schedule {
        Schedule::Every(Duration::from_secs(300))
    }

    async fn run(&self, app: &App, now: DateTime<Utc>) -> AppResult<()> {
        let stats = app.run_auto_close(now).await?;
        if stats.processed > 0 {
            tracing::info!(processed = stats.processed, "auto-closed issues");
        }
        if !stats.errors.is_empty() {
            tracing::warn!(errors = ?stats.errors, "auto-close finished with errors");
        }
        Ok(())
    }
}
