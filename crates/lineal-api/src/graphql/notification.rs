//! GraphQL-Teilobjekte des Bereichs `notification` (WP08).
//!
//! Das Platzhalterfeld `_notificationReady` ist sichtbar (nicht versteckt), damit der Root-Typ im
//! exportierten SDL nie leer ist. Das zuständige Arbeitspaket löscht es, sobald der Bereich
//! echte Felder hat.

use async_graphql::Object;

#[derive(Default)]
pub struct NotificationQuery;

#[Object]
impl NotificationQuery {
    /// Platzhalter des Scaffolds.
    #[graphql(name = "_notificationReady")]
    async fn notification_ready(&self) -> bool {
        true
    }
}

#[derive(Default)]
pub struct NotificationMutation;

#[Object]
impl NotificationMutation {
    /// Platzhalter des Scaffolds.
    #[graphql(name = "_notificationMutationReady")]
    async fn notification_mutation_ready(&self) -> bool {
        true
    }
}
