//! GraphQL-Teilobjekte des Bereichs `org` (WP02).
//!
//! Das Platzhalterfeld `_orgReady` ist sichtbar (nicht versteckt), damit der Root-Typ im
//! exportierten SDL nie leer ist. Das zuständige Arbeitspaket löscht es, sobald der Bereich
//! echte Felder hat.

use async_graphql::Object;

#[derive(Default)]
pub struct OrgQuery;

#[Object]
impl OrgQuery {
    /// Platzhalter des Scaffolds.
    #[graphql(name = "_orgReady")]
    async fn org_ready(&self) -> bool {
        true
    }
}

#[derive(Default)]
pub struct OrgMutation;

#[Object]
impl OrgMutation {
    /// Platzhalter des Scaffolds.
    #[graphql(name = "_orgMutationReady")]
    async fn org_mutation_ready(&self) -> bool {
        true
    }
}
