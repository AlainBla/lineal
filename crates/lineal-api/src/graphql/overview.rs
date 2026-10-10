//! GraphQL-Teilobjekte des Bereichs `overview` (WP11).
//!
//! Das Platzhalterfeld `_overviewReady` ist sichtbar (nicht versteckt), damit der Root-Typ im
//! exportierten SDL nie leer ist. Das zuständige Arbeitspaket löscht es, sobald der Bereich
//! echte Felder hat.

use async_graphql::Object;

#[derive(Default)]
pub struct OverviewQuery;

#[Object]
impl OverviewQuery {
    /// Platzhalter des Scaffolds.
    #[graphql(name = "_overviewReady")]
    async fn overview_ready(&self) -> bool {
        true
    }
}

#[derive(Default)]
pub struct OverviewMutation;

#[Object]
impl OverviewMutation {
    /// Platzhalter des Scaffolds.
    #[graphql(name = "_overviewMutationReady")]
    async fn overview_mutation_ready(&self) -> bool {
        true
    }
}
