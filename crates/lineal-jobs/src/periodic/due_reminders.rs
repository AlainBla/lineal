//! Periodischer Job `due_reminders` (WP08). Im Scaffold ein No-op.

use crate::{PeriodicJob, Schedule};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_app::{App, AppResult};
use std::time::Duration;

pub struct DueRemindersJob;

#[async_trait]
impl PeriodicJob for DueRemindersJob {
    fn name(&self) -> &'static str {
        "due_reminders"
    }

    fn schedule(&self) -> Schedule {
        Schedule::Every(Duration::from_secs(300))
    }

    async fn run(&self, _app: &App, _now: DateTime<Utc>) -> AppResult<()> {
        Ok(())
    }
}
