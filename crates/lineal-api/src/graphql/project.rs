//! GraphQL-Teilobjekte des Bereichs `project` (WP07).
//!
//! Das Platzhalterfeld `_projectReady` ist sichtbar (nicht versteckt), damit der Root-Typ im
//! exportierten SDL nie leer ist. Das zuständige Arbeitspaket löscht es, sobald der Bereich
//! echte Felder hat.

use async_graphql::Object;

#[derive(Default)]
pub struct ProjectQuery;

#[Object]
impl ProjectQuery {
    /// Platzhalter des Scaffolds.
    #[graphql(name = "_projectReady")]
    async fn project_ready(&self) -> bool {
        true
    }
}

#[derive(Default)]
pub struct ProjectMutation;

#[Object]
impl ProjectMutation {
    /// Platzhalter des Scaffolds.
    #[graphql(name = "_projectMutationReady")]
    async fn project_mutation_ready(&self) -> bool {
        true
    }
}
