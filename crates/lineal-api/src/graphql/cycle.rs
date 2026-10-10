//! GraphQL-Teilobjekte des Bereichs `cycle` (WP07).
//!
//! Das Platzhalterfeld `_cycleReady` ist sichtbar (nicht versteckt), damit der Root-Typ im
//! exportierten SDL nie leer ist. Das zuständige Arbeitspaket löscht es, sobald der Bereich
//! echte Felder hat.

use async_graphql::Object;

#[derive(Default)]
pub struct CycleQuery;

#[Object]
impl CycleQuery {
    /// Platzhalter des Scaffolds.
    #[graphql(name = "_cycleReady")]
    async fn cycle_ready(&self) -> bool {
        true
    }
}

#[derive(Default)]
pub struct CycleMutation;

#[Object]
impl CycleMutation {
    /// Platzhalter des Scaffolds.
    #[graphql(name = "_cycleMutationReady")]
    async fn cycle_mutation_ready(&self) -> bool {
        true
    }
}
