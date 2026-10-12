//! View-Abos (docs/05 §4.7): Schnittstelle zwischen dem Notification-Fan-out (WP32) und dem
//! Datenmodell der View-Abonnements (WP31).
//!
//! Der Fan-out kennt keine View-Filter. Er fragt für jedes Issue-Ereignis eine
//! [`ViewSubscriptionSource`], welche Abonnenten abonnierter Views das Ereignis betrifft, und
//! liefert die Notifications selbst (Self-Suppression, Sichtbarkeit je Empfänger, Bündelung,
//! Idempotenz). Die Quelle entscheidet nur, **wer** welchen Typ bekommt:
//!
//! | Typ | Bedingung |
//! |---|---|
//! | `issueAddedToView` | das Issue erfüllt den View-Filter jetzt, vorher nicht (oder wurde neu angelegt) |
//! | `issueStatusChanged` | ein Issue der View wechselt nach `completed`/`canceled` |
//! | `issueStatusChangedAll` | jeder Statuswechsel eines Issues der View |
//!
//! Die Filterauswertung läuft mit den Rechten des Abonnenten. Als Vorher-Werte stehen
//! [`ViewIssueEvent::previous`] (Assignee, Statustyp) zur Verfügung.
//!
//! **Anbindung bei der Zusammenführung mit WP31:** `ViewSubscriptionSource` implementieren
//! (gestützt auf `notification_subscription.custom_view_id` und den View-Filter) und im
//! Konsumenten einhängen: `NotificationsConsumer::with_views(Arc<..>)`
//! (`crates/lineal-jobs/src/consumers/notifications.rs`). Ohne Quelle entstehen keine
//! View-Benachrichtigungen; alles andere läuft unverändert.

use crate::error::AppResult;
use crate::ports::Tx;
use crate::ports::repos::notification::IssueFacts;
use async_trait::async_trait;
use lineal_domain::events::IssuePrevious;
use lineal_domain::ids::UserId;

/// Ein Issue-Ereignis aus Sicht der View-Abos.
#[derive(Debug, Clone, Copy)]
pub struct ViewIssueEvent<'a> {
    /// Zustand des Issues nach dem Ereignis.
    pub issue: &'a IssueFacts,
    /// Das Issue wurde gerade angelegt.
    pub is_new: bool,
    /// Geänderte Felder (camelCase, `IssueUpdated.changed`); bei neuen Issues leer.
    pub changed: &'a [String],
    /// Alte Werte (Assignee, Statustyp), soweit erfasst.
    pub previous: Option<&'a IssuePrevious>,
}

/// Liefert die Empfänger von View-Benachrichtigungen zu einem Issue-Ereignis.
#[async_trait]
pub trait ViewSubscriptionSource: Send + Sync {
    /// Paare aus Abonnent und Notification-Typ (`issueAddedToView`, `issueStatusChanged`,
    /// `issueStatusChangedAll`). Mehrfache Nennungen sind erlaubt; es gewinnt der spezifischste
    /// Typ je Empfänger. Der Akteur und nicht sichtbare Empfänger werden vom Fan-out entfernt.
    async fn recipients(
        &self,
        tx: &mut dyn Tx,
        event: &ViewIssueEvent<'_>,
    ) -> AppResult<Vec<(UserId, &'static str)>>;
}
