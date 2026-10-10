//! GraphQL-Teilobjekte des Bereichs `team` (WP02).
//!
//! Das Platzhalterfeld `_teamReady` ist sichtbar (nicht versteckt), damit der Root-Typ im
//! exportierten SDL nie leer ist. Das zuständige Arbeitspaket löscht es, sobald der Bereich
//! echte Felder hat.

use async_graphql::Object;

#[derive(Default)]
pub struct TeamQuery;

#[Object]
impl TeamQuery {
    /// Platzhalter des Scaffolds.
    #[graphql(name = "_teamReady")]
    async fn team_ready(&self) -> bool {
        true
    }
}

#[derive(Default)]
pub struct TeamMutation;

#[Object]
impl TeamMutation {
    /// Platzhalter des Scaffolds.
    #[graphql(name = "_teamMutationReady")]
    async fn team_mutation_ready(&self) -> bool {
        true
    }
}
