//! GraphQL-Teilobjekte des Bereichs `view` (WP11).
//!
//! Das Platzhalterfeld `_viewReady` ist sichtbar (nicht versteckt), damit der Root-Typ im
//! exportierten SDL nie leer ist. Das zuständige Arbeitspaket löscht es, sobald der Bereich
//! echte Felder hat.

use async_graphql::Object;

#[derive(Default)]
pub struct ViewQuery;

#[Object]
impl ViewQuery {
    /// Platzhalter des Scaffolds.
    #[graphql(name = "_viewReady")]
    async fn view_ready(&self) -> bool {
        true
    }
}

#[derive(Default)]
pub struct ViewMutation;

#[Object]
impl ViewMutation {
    /// Platzhalter des Scaffolds.
    #[graphql(name = "_viewMutationReady")]
    async fn view_mutation_ready(&self) -> bool {
        true
    }
}
