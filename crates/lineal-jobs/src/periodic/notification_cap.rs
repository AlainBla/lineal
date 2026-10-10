//! Periodischer Job `notification_cap` (WP08): höchstens `maxUnarchivedNotificationsPerUser`
//! (Default 2000) nicht archivierte Notifications je User; entfernt außerdem archivierte
//! Notifications nach 90 Tagen.

use crate::{PeriodicJob, Schedule};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_app::{App, AppResult};
use std::time::Duration;

pub struct NotificationCapJob;

#[async_trait]
impl PeriodicJob for NotificationCapJob {
    fn name(&self) -> &'static str {
        "notification_cap"
    }

    fn schedule(&self) -> Schedule {
        Schedule::Every(Duration::from_secs(300))
    }

    async fn run(&self, app: &App, now: DateTime<Utc>) -> AppResult<()> {
        let archived = app.enforce_notification_cap(now).await?;
        if archived > 0 {
            tracing::info!(archived, "notification cap enforced");
        }
        Ok(())
    }
}
