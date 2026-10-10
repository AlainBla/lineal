//! Periodischer Job `email_digest` (WP08): versendet fällige E-Mail-Digests (docs/05 §6.9).
//! Verzögerung, Arbeitszeit und Kanal-Einstellungen wertet `App::send_email_digests` aus.

use crate::{PeriodicJob, Schedule};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_app::{App, AppResult};
use std::time::Duration;

pub struct EmailDigestJob;

#[async_trait]
impl PeriodicJob for EmailDigestJob {
    fn name(&self) -> &'static str {
        "email_digest"
    }

    fn schedule(&self) -> Schedule {
        Schedule::Every(Duration::from_secs(60))
    }

    async fn run(&self, app: &App, now: DateTime<Utc>) -> AppResult<()> {
        let sent = app.send_email_digests(now).await?;
        if sent > 0 {
            tracing::info!(sent, "email digests sent");
        }
        Ok(())
    }
}
