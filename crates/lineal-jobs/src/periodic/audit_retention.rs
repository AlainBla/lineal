//! Periodischer Job `audit_retention` (WP25, docs/15 §4.7): löscht Audit-Log-Einträge, die älter
//! als die Workspace-Einstellung `auditLogRetentionDays` sind (Default 365, Minimum 30). Die
//! Löschung läuft über die SQL-Funktion der Migration 2500; sonst ist das Log append-only.

use crate::{PeriodicJob, Schedule};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_app::{App, AppResult};
use std::time::Duration;

pub struct AuditRetentionJob;

#[async_trait]
impl PeriodicJob for AuditRetentionJob {
    fn name(&self) -> &'static str {
        "audit_retention"
    }

    fn schedule(&self) -> Schedule {
        Schedule::Every(Duration::from_secs(3600))
    }

    async fn run(&self, app: &App, now: DateTime<Utc>) -> AppResult<()> {
        let deleted = app.purge_audit_log(now).await?;
        if deleted > 0 {
            tracing::info!(entries = deleted, "audit log entries purged");
        }
        Ok(())
    }
}
