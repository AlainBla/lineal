//! GraphQL-Teilobjekte des Bereichs `search` (WP11).
//!
//! Das Platzhalterfeld `_searchReady` ist sichtbar (nicht versteckt), damit der Root-Typ im
//! exportierten SDL nie leer ist. Das zuständige Arbeitspaket löscht es, sobald der Bereich
//! echte Felder hat.

use async_graphql::Object;

#[derive(Default)]
pub struct SearchQuery;

#[Object]
impl SearchQuery {
    /// Platzhalter des Scaffolds.
    #[graphql(name = "_searchReady")]
    async fn search_ready(&self) -> bool {
        true
    }
}

#[derive(Default)]
pub struct SearchMutation;

#[Object]
impl SearchMutation {
    /// Platzhalter des Scaffolds.
    #[graphql(name = "_searchMutationReady")]
    async fn search_mutation_ready(&self) -> bool {
        true
    }
}
