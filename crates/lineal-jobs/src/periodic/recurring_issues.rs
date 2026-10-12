//! Periodischer Job `recurring_issues` (WP30): legt aus fälligen wiederkehrenden Issues
//! (docs/03 §13) Issues an. Die Logik liegt in `App::run_recurring_issues`; der Lauf ist
//! idempotent (`recurring_issue_run`).

use crate::{PeriodicJob, Schedule};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_app::{App, AppResult};
use std::time::Duration;

pub struct RecurringIssuesJob;

#[async_trait]
impl PeriodicJob for RecurringIssuesJob {
    fn name(&self) -> &'static str {
        "recurring_issues"
    }

    fn schedule(&self) -> Schedule {
        Schedule::Every(Duration::from_secs(60))
    }

    async fn run(&self, app: &App, now: DateTime<Utc>) -> AppResult<()> {
        let created = app.run_recurring_issues(now).await?;
        if created > 0 {
            tracing::info!(created, "recurring issues processed");
        }
        Ok(())
    }
}
