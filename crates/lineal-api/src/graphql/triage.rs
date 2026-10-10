//! GraphQL-Teilobjekte des Bereichs `triage` (WP12).
//!
//! Das Platzhalterfeld `_triageReady` ist sichtbar (nicht versteckt), damit der Root-Typ im
//! exportierten SDL nie leer ist. Das zuständige Arbeitspaket löscht es, sobald der Bereich
//! echte Felder hat.

use async_graphql::Object;

#[derive(Default)]
pub struct TriageQuery;

#[Object]
impl TriageQuery {
    /// Platzhalter des Scaffolds.
    #[graphql(name = "_triageReady")]
    async fn triage_ready(&self) -> bool {
        true
    }
}

#[derive(Default)]
pub struct TriageMutation;

#[Object]
impl TriageMutation {
    /// Platzhalter des Scaffolds.
    #[graphql(name = "_triageMutationReady")]
    async fn triage_mutation_ready(&self) -> bool {
        true
    }
}
