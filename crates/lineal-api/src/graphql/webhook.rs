//! GraphQL-Teilobjekte des Bereichs `webhook` (WP16).
//!
//! Das Platzhalterfeld `_webhookReady` ist sichtbar (nicht versteckt), damit der Root-Typ im
//! exportierten SDL nie leer ist. Das zuständige Arbeitspaket löscht es, sobald der Bereich
//! echte Felder hat.

use async_graphql::Object;

#[derive(Default)]
pub struct WebhookQuery;

#[Object]
impl WebhookQuery {
    /// Platzhalter des Scaffolds.
    #[graphql(name = "_webhookReady")]
    async fn webhook_ready(&self) -> bool {
        true
    }
}

#[derive(Default)]
pub struct WebhookMutation;

#[Object]
impl WebhookMutation {
    /// Platzhalter des Scaffolds.
    #[graphql(name = "_webhookMutationReady")]
    async fn webhook_mutation_ready(&self) -> bool {
        true
    }
}
