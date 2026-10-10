//! GraphQL-Teilobjekte des Bereichs `issue` (WP06).
//!
//! Das Platzhalterfeld `_issueReady` ist sichtbar (nicht versteckt), damit der Root-Typ im
//! exportierten SDL nie leer ist. Das zuständige Arbeitspaket löscht es, sobald der Bereich
//! echte Felder hat.

use async_graphql::Object;

#[derive(Default)]
pub struct IssueQuery;

#[Object]
impl IssueQuery {
    /// Platzhalter des Scaffolds.
    #[graphql(name = "_issueReady")]
    async fn issue_ready(&self) -> bool {
        true
    }
}

#[derive(Default)]
pub struct IssueMutation;

#[Object]
impl IssueMutation {
    /// Platzhalter des Scaffolds.
    #[graphql(name = "_issueMutationReady")]
    async fn issue_mutation_ready(&self) -> bool {
        true
    }
}
