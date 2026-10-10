//! GraphQL-Teilobjekte des Bereichs `initiative` (WP07).
//!
//! Das Platzhalterfeld `_initiativeReady` ist sichtbar (nicht versteckt), damit der Root-Typ im
//! exportierten SDL nie leer ist. Das zuständige Arbeitspaket löscht es, sobald der Bereich
//! echte Felder hat.

use async_graphql::Object;

#[derive(Default)]
pub struct InitiativeQuery;

#[Object]
impl InitiativeQuery {
    /// Platzhalter des Scaffolds.
    #[graphql(name = "_initiativeReady")]
    async fn initiative_ready(&self) -> bool {
        true
    }
}

#[derive(Default)]
pub struct InitiativeMutation;

#[Object]
impl InitiativeMutation {
    /// Platzhalter des Scaffolds.
    #[graphql(name = "_initiativeMutationReady")]
    async fn initiative_mutation_ready(&self) -> bool {
        true
    }
}
