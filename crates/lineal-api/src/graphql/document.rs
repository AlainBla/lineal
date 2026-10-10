//! GraphQL-Teilobjekte des Bereichs `document` (WP10).
//!
//! Das Platzhalterfeld `_documentReady` ist sichtbar (nicht versteckt), damit der Root-Typ im
//! exportierten SDL nie leer ist. Das zuständige Arbeitspaket löscht es, sobald der Bereich
//! echte Felder hat.

use async_graphql::Object;

#[derive(Default)]
pub struct DocumentQuery;

#[Object]
impl DocumentQuery {
    /// Platzhalter des Scaffolds.
    #[graphql(name = "_documentReady")]
    async fn document_ready(&self) -> bool {
        true
    }
}

#[derive(Default)]
pub struct DocumentMutation;

#[Object]
impl DocumentMutation {
    /// Platzhalter des Scaffolds.
    #[graphql(name = "_documentMutationReady")]
    async fn document_mutation_ready(&self) -> bool {
        true
    }
}
