//! Periodischer Job `reminders` (F.10): löst fällige „Remind me“-Erinnerungen aus
//! (docs/05 §6.10) und legt Notifications `issueReminder`, `projectReminder`,
//! `initiativeReminder` bzw. `documentReminder` an. Läuft jede Minute.

use crate::{PeriodicJob, Schedule};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_app::{App, AppResult};
use std::time::Duration;

pub struct RemindersJob;

#[async_trait]
impl PeriodicJob for RemindersJob {
    fn name(&self) -> &'static str {
        "reminders"
    }

    fn schedule(&self) -> Schedule {
        Schedule::Every(Duration::from_secs(60))
    }

    async fn run(&self, app: &App, now: DateTime<Utc>) -> AppResult<()> {
        let created = app.create_reminder_notifications(now).await?;
        if created > 0 {
            tracing::info!(created, "reminder notifications");
        }
        Ok(())
    }
}
