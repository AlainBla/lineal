//! Periodischer Job `totp_grace` (LIN-70): widerruft die Sessions von Org-Ownern,
//! deren TOTP-Schonfrist ohne bestätigtes 2FA abgelaufen ist. Ohne diesen Job liefe
//! eine bestehende Session bis zum Ablauf (14 Tage) weiter und die „harte Sperre"
//! der Schonfrist (7 Tage / 3 Logins) griffe nicht. Die Logik liegt in
//! `App::enforce_totp_grace`.

use crate::{PeriodicJob, Schedule};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_app::{App, AppResult};
use std::time::Duration;

pub struct TotpGraceJob;

#[async_trait]
impl PeriodicJob for TotpGraceJob {
    fn name(&self) -> &'static str {
        "totp_grace"
    }

    fn schedule(&self) -> Schedule {
        Schedule::Every(Duration::from_secs(3600))
    }

    async fn run(&self, app: &App, now: DateTime<Utc>) -> AppResult<()> {
        let revoked = app.enforce_totp_grace(now).await?;
        if revoked > 0 {
            tracing::info!(
                owners = revoked,
                "revoked sessions of owners past the 2fa grace period"
            );
        }
        Ok(())
    }
}
