//! Periodischer Job `cycles` (WP07). Im Scaffold ein No-op.

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
        Schedule::Every(Duration::from_secs(300))
    }

    async fn run(&self, _app: &App, _now: DateTime<Utc>) -> AppResult<()> {
        Ok(())
    }
}
