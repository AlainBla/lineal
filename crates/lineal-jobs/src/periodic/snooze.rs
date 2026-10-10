//! Periodischer Job `snooze` (WP08): Wiedervorlage gesnoozter Notifications (docs/05 §6.4 Nr. 3).

use crate::{PeriodicJob, Schedule};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_app::{App, AppResult};
use std::time::Duration;

pub struct SnoozeJob;

#[async_trait]
impl PeriodicJob for SnoozeJob {
    fn name(&self) -> &'static str {
        "snooze"
    }

    fn schedule(&self) -> Schedule {
        Schedule::Every(Duration::from_secs(60))
    }

    async fn run(&self, app: &App, now: DateTime<Utc>) -> AppResult<()> {
        let n = app.unsnooze_due_notifications(now).await?;
        if n > 0 {
            tracing::info!(notifications = n, "snoozed notifications resurfaced");
        }
        Ok(())
    }
}
