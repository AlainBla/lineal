//! Periodischer Job `due_reminders` (WP08): `issueDue`-Erinnerungen (docs/05 §6.10) und der
//! E-Mail-Digest der Inbox (docs/05 §6.9). Beide laufen alle 5 Minuten; der Digest hat keinen
//! eigenen Registry-Eintrag und hängt deshalb hier.

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

    async fn run(&self, app: &App, now: DateTime<Utc>) -> AppResult<()> {
        let due = app.create_due_reminders(now).await;
        // Der Digest läuft auch, wenn die Erinnerungen scheitern.
        let mails = app.send_email_digests(now).await;
        let created = due?;
        let sent = mails?;
        if created + sent > 0 {
            tracing::info!(created, sent, "due reminders and email digests");
        }
        Ok(())
    }
}
