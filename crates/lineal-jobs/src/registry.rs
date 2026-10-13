//! Liste aller Konsumenten und periodischen Jobs. Gemeinsame Datei: nur S1 / Orchestrator.

use crate::consumers::*;
use crate::periodic::*;
use crate::{EventConsumer, PeriodicJob};
use lineal_app::usecases::notification::views::ViewSubscriptions;
use std::sync::Arc;

/// Alle Outbox-Konsumenten.
pub fn consumers() -> Vec<Arc<dyn EventConsumer>> {
    vec![
        Arc::new(notifications::NotificationsWithViewsConsumer {
            views: Arc::new(ViewSubscriptions),
        }),
        Arc::new(search_index::SearchIndexConsumer),
        Arc::new(sync::SyncConsumer),
        Arc::new(webhooks::WebhooksConsumer),
    ]
}

/// Alle periodischen Jobs.
pub fn periodic_jobs() -> Vec<Arc<dyn PeriodicJob>> {
    vec![
        Arc::new(cycles::CyclesJob),
        Arc::new(update_reminders::UpdateRemindersJob),
        Arc::new(snooze::SnoozeJob),
        Arc::new(sync_purge::SyncPurgeJob),
        Arc::new(notification_cap::NotificationCapJob),
        Arc::new(due_reminders::DueRemindersJob),
        Arc::new(email_digest::EmailDigestJob),
        Arc::new(auto_close::AutoCloseJob),
        Arc::new(auto_archive::AutoArchiveJob),
        Arc::new(purge::PurgeJob),
        Arc::new(sla::SlaJob),
        Arc::new(triage_rotation::TriageRotationJob),
        Arc::new(import_runner::ImportRunnerJob),
        Arc::new(webhooks::WebhookDeliveryJob),
        Arc::new(team_purge::TeamPurgeJob),
        Arc::new(audit_retention::AuditRetentionJob),
        Arc::new(recurring_issues::RecurringIssuesJob),
        Arc::new(sort_renumber::SortRenumberJob),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn names_are_unique() {
        let c: HashSet<_> = consumers().iter().map(|c| c.name()).collect();
        assert_eq!(c.len(), consumers().len());
        let p: HashSet<_> = periodic_jobs().iter().map(|p| p.name()).collect();
        assert_eq!(p.len(), periodic_jobs().len());
    }
}
